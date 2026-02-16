//! Win32 `SendInput` mouse and keyboard injection.
//!
//! Used by the slave to replay input events received from the master.
//!
//! # Platform
//!
//! Windows-only. On other platforms the injector is defined but all
//! methods return an error.

use crate::error::TixError;

// ── InputInjector ────────────────────────────────────────────────

/// Injects mouse and keyboard events into the OS input stream.
///
/// On Windows this uses `SendInput` which requires the calling
/// process to be running in the same desktop session (or with
/// `UIAccess` privileges).
pub struct InputInjector;

impl InputInjector {
    /// Create a new injector (no initialisation cost).
    pub fn new() -> Self {
        Self
    }
}

impl Default for InputInjector {
    fn default() -> Self {
        Self::new()
    }
}

// ── Windows implementation ───────────────────────────────────────

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use crate::protocol::screen::{KeyAction, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    impl InputInjector {
        /// Inject a mouse event from the TixRP protocol.
        pub fn inject_mouse(&self, event: &MouseEvent) -> Result<(), TixError> {
            // Convert to absolute coordinates (0..65535).
            let (screen_w, screen_h) = unsafe {
                use windows::Win32::UI::WindowsAndMessaging::*;
                let w = GetSystemMetrics(SM_CXSCREEN);
                let h = GetSystemMetrics(SM_CYSCREEN);
                (w, h)
            };

            if screen_w == 0 || screen_h == 0 {
                return Err(TixError::Other("GetSystemMetrics returned 0".into()));
            }

            let abs_x = (event.x as i64 * 65535 / screen_w as i64) as i32;
            let abs_y = (event.y as i64 * 65535 / screen_h as i64) as i32;

            let mut flags = MOUSE_EVENT_FLAGS(0);
            let mut mouse_data: u32 = 0;

            match event.kind {
                MouseEventKind::Move => {
                    flags |= MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE;
                }
                MouseEventKind::Press => {
                    flags |= MOUSEEVENTF_ABSOLUTE;
                    flags |= match event.button {
                        MouseButton::Left => MOUSEEVENTF_LEFTDOWN,
                        MouseButton::Right => MOUSEEVENTF_RIGHTDOWN,
                        MouseButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
                        MouseButton::X1 => {
                            mouse_data = 1; // XBUTTON1
                            MOUSEEVENTF_XDOWN
                        }
                        MouseButton::X2 => {
                            mouse_data = 2; // XBUTTON2
                            MOUSEEVENTF_XDOWN
                        }
                        MouseButton::None => MOUSE_EVENT_FLAGS(0),
                    };
                }
                MouseEventKind::Release => {
                    flags |= MOUSEEVENTF_ABSOLUTE;
                    flags |= match event.button {
                        MouseButton::Left => MOUSEEVENTF_LEFTUP,
                        MouseButton::Right => MOUSEEVENTF_RIGHTUP,
                        MouseButton::Middle => MOUSEEVENTF_MIDDLEUP,
                        MouseButton::X1 => {
                            mouse_data = 1;
                            MOUSEEVENTF_XUP
                        }
                        MouseButton::X2 => {
                            mouse_data = 2;
                            MOUSEEVENTF_XUP
                        }
                        MouseButton::None => MOUSE_EVENT_FLAGS(0),
                    };
                }
                MouseEventKind::Scroll => {
                    flags |= MOUSEEVENTF_WHEEL | MOUSEEVENTF_ABSOLUTE;
                    mouse_data = event.scroll_delta as u16 as u32;
                }
                MouseEventKind::DoubleClick => {
                    // Synthesize a double-click as down-up-down-up.
                    let down = MouseEvent::press(event.x, event.y, event.button);
                    let up = MouseEvent::release(event.x, event.y, event.button);
                    self.inject_mouse(&down)?;
                    self.inject_mouse(&up)?;
                    self.inject_mouse(&down)?;
                    self.inject_mouse(&up)?;
                    return Ok(());
                }
            }

            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: abs_x,
                        dy: abs_y,
                        mouseData: mouse_data,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
            if sent == 0 {
                return Err(TixError::Other("SendInput (mouse) returned 0".into()));
            }

            Ok(())
        }

        /// Inject a keyboard event from the TixRP protocol.
        pub fn inject_keyboard(&self, event: &KeyEvent) -> Result<(), TixError> {
            let mut flags = KEYBD_EVENT_FLAGS(0);

            // Use scan code if available, otherwise virtual key.
            if event.scan_code != 0 {
                flags |= KEYEVENTF_SCANCODE;
            }

            if event.action == KeyAction::Release {
                flags |= KEYEVENTF_KEYUP;
            }

            // Extended keys (right Ctrl, right Alt, arrow keys, etc.)
            // have scan codes with 0xE0 prefix.
            if event.scan_code > 0xFF {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }

            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(event.virtual_key),
                        wScan: event.scan_code,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
            if sent == 0 {
                return Err(TixError::Other("SendInput (keyboard) returned 0".into()));
            }

            Ok(())
        }
    }
}

// ── Non-Windows stub ─────────────────────────────────────────────

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::*;
    use crate::protocol::screen::{KeyEvent, MouseEvent};

    impl InputInjector {
        pub fn inject_mouse(&self, _event: &MouseEvent) -> Result<(), TixError> {
            Err(TixError::Other(
                "Input injection is only available on Windows".into(),
            ))
        }

        pub fn inject_keyboard(&self, _event: &KeyEvent) -> Result<(), TixError> {
            Err(TixError::Other(
                "Input injection is only available on Windows".into(),
            ))
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injector_creates_without_error() {
        let _inj = InputInjector::new();
    }
}
