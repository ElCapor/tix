//! Win32 window creation and message loop.
//!
//! Creates a native HWND used by the display renderer. The window
//! produces [`WindowEvent`]s that the main loop processes for input
//! forwarding and lifecycle management.

#[cfg(target_os = "windows")]
mod platform {
    use std::sync::mpsc;

    use windows::Win32::Foundation::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::PCWSTR;

    /// Events produced by the window message loop.
    #[derive(Debug, Clone)]
    pub enum WindowEvent {
        /// Window close requested (Alt-F4/X button).
        Close,
        /// Window resized.
        Resize(u32, u32),
        /// Mouse moved (client-relative coordinates).
        MouseMove(i32, i32),
        /// Mouse button pressed or released.
        MouseButton(MouseBtn, bool),
        /// Mouse wheel delta.
        MouseWheel(i16),
        /// Key down/up: virtual-key code, scan code, pressed.
        Key(u16, u16, bool),
    }

    /// Mouse button identifiers.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MouseBtn {
        Left,
        Right,
        Middle,
    }

    /// Handle to the native window.
    pub struct NativeWindow {
        pub hwnd: HWND,
        pub width: u32,
        pub height: u32,
        event_rx: mpsc::Receiver<WindowEvent>,
    }

    // We store a raw pointer to the mpsc sender in GWLP_USERDATA.
    // This is safe because the pointer lives as long as the window.
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let tx_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const mpsc::Sender<WindowEvent>;

        if tx_ptr.is_null() {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }

        let tx = unsafe { &*tx_ptr };

        match msg {
            WM_CLOSE => {
                let _ = tx.send(WindowEvent::Close);
                LRESULT(0)
            }
            WM_SIZE => {
                let w = (lparam.0 & 0xFFFF) as u32;
                let h = ((lparam.0 >> 16) & 0xFFFF) as u32;
                let _ = tx.send(WindowEvent::Resize(w, h));
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let _ = tx.send(WindowEvent::MouseMove(x, y));
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let _ = tx.send(WindowEvent::MouseButton(MouseBtn::Left, true));
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = tx.send(WindowEvent::MouseButton(MouseBtn::Left, false));
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                let _ = tx.send(WindowEvent::MouseButton(MouseBtn::Right, true));
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let _ = tx.send(WindowEvent::MouseButton(MouseBtn::Right, false));
                LRESULT(0)
            }
            WM_MBUTTONDOWN => {
                let _ = tx.send(WindowEvent::MouseButton(MouseBtn::Middle, true));
                LRESULT(0)
            }
            WM_MBUTTONUP => {
                let _ = tx.send(WindowEvent::MouseButton(MouseBtn::Middle, false));
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
                let _ = tx.send(WindowEvent::MouseWheel(delta));
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = (wparam.0 & 0xFFFF) as u16;
                let scan = ((lparam.0 >> 16) & 0xFF) as u16;
                let _ = tx.send(WindowEvent::Key(vk, scan, true));
                LRESULT(0)
            }
            WM_KEYUP | WM_SYSKEYUP => {
                let vk = (wparam.0 & 0xFFFF) as u16;
                let scan = ((lparam.0 >> 16) & 0xFF) as u16;
                let _ = tx.send(WindowEvent::Key(vk, scan, false));
                LRESULT(0)
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    impl NativeWindow {
        /// Create a new top-level window.
        pub fn create(title: &str, width: u32, height: u32) -> Result<Self, String> {
            let (event_tx, event_rx) = mpsc::channel();

            let hinstance = unsafe { GetModuleHandleW(None) }
                .map_err(|e| format!("GetModuleHandle: {e}"))?;

            let class_name_wide: Vec<u16> = "TixRdpGuiClass\0"
                .encode_utf16()
                .collect();

            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: PCWSTR(class_name_wide.as_ptr()),
                hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
                ..Default::default()
            };

            let atom = unsafe { RegisterClassW(&wc) };
            if atom == 0 {
                return Err("RegisterClassW failed".into());
            }

            let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();

            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR(class_name_wide.as_ptr()),
                    PCWSTR(title_wide.as_ptr()),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    width as i32,
                    height as i32,
                    None,
                    None,
                    hinstance,
                    None,
                )
            }.map_err(|e| format!("CreateWindowExW failed: {e}"))?;

            if hwnd.is_invalid() {
                return Err("CreateWindowExW returned invalid HWND".into());
            }

            // Store the event sender pointer in GWLP_USERDATA.
            let tx_box = Box::new(event_tx);
            let tx_ptr = Box::into_raw(tx_box);
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, tx_ptr as isize);
            }

            Ok(Self {
                hwnd,
                width,
                height,
                event_rx,
            })
        }

        /// Pump windows messages (non-blocking). Returns collected events.
        pub fn poll_events(&self) -> Vec<WindowEvent> {
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, self.hwnd, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            let mut events = Vec::new();
            while let Ok(ev) = self.event_rx.try_recv() {
                events.push(ev);
            }
            events
        }

        /// The raw window handle.
        pub fn hwnd(&self) -> HWND {
            self.hwnd
        }
    }

    impl Drop for NativeWindow {
        fn drop(&mut self) {
            unsafe {
                // Recover and drop the boxed sender.
                let ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA)
                    as *mut mpsc::Sender<WindowEvent>;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                    SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                }
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub use platform::*;

// ── Non-Windows stub ─────────────────────────────────────────────

#[cfg(not(target_os = "windows"))]
pub mod stub {
    #[derive(Debug, Clone)]
    pub enum WindowEvent {
        Close,
        Resize(u32, u32),
        MouseMove(i32, i32),
        MouseButton(MouseBtn, bool),
        MouseWheel(i16),
        Key(u16, u16, bool),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MouseBtn {
        Left,
        Right,
        Middle,
    }

    pub struct NativeWindow;

    impl NativeWindow {
        pub fn create(_title: &str, _w: u32, _h: u32) -> Result<Self, String> {
            Err("Window creation is only supported on Windows".into())
        }

        pub fn poll_events(&self) -> Vec<WindowEvent> {
            Vec::new()
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub::*;
