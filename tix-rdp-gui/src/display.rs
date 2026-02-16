//! Display renderer — blits decoded frame buffers to the window.
//!
//! Uses GDI `StretchDIBits` for maximum compatibility. A future
//! iteration could use Direct3D 11 for GPU-accelerated rendering.

#[cfg(target_os = "windows")]
mod platform {
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;

    /// Renders BGRA8 frame buffers into an HWND using GDI.
    pub struct DisplayRenderer {
        hwnd: HWND,
        width: u32,
        height: u32,
    }

    impl DisplayRenderer {
        /// Create a renderer targeting the given window.
        pub fn new(hwnd: HWND, width: u32, height: u32) -> Self {
            Self { hwnd, width, height }
        }

        /// Update the target size (call after WM_SIZE).
        pub fn resize(&mut self, width: u32, height: u32) {
            self.width = width;
            self.height = height;
        }

        /// Render a BGRA8 frame buffer to the window.
        ///
        /// `frame_width` / `frame_height` describe the pixel dimensions
        /// of `data`. The image is stretched to fill the window.
        pub fn render(
            &self,
            data: &[u8],
            frame_width: u32,
            frame_height: u32,
        ) -> Result<(), String> {
            if data.is_empty() {
                return Ok(());
            }

            let expected = (frame_width * frame_height * 4) as usize;
            if data.len() < expected {
                return Err(format!(
                    "frame buffer too small: {} < {}",
                    data.len(),
                    expected,
                ));
            }

            unsafe {
                let hdc = GetDC(self.hwnd);
                if hdc.is_invalid() {
                    return Err("GetDC failed".into());
                }

                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: frame_width as i32,
                        // Negative height = top-down DIB (origin at top-left).
                        biHeight: -(frame_height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        biSizeImage: 0,
                        biXPelsPerMeter: 0,
                        biYPelsPerMeter: 0,
                        biClrUsed: 0,
                        biClrImportant: 0,
                    },
                    bmiColors: [RGBQUAD::default(); 1],
                };

                StretchDIBits(
                    hdc,
                    0,
                    0,
                    self.width as i32,
                    self.height as i32,
                    0,
                    0,
                    frame_width as i32,
                    frame_height as i32,
                    Some(data.as_ptr() as *const _),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );

                ReleaseDC(self.hwnd, hdc);
            }

            Ok(())
        }
    }
}

#[cfg(target_os = "windows")]
pub use platform::*;

// ── Non-Windows stub ─────────────────────────────────────────────

#[cfg(not(target_os = "windows"))]
pub mod stub {
    pub struct DisplayRenderer;

    impl DisplayRenderer {
        pub fn new(_hwnd: (), _w: u32, _h: u32) -> Self {
            Self
        }

        pub fn resize(&mut self, _w: u32, _h: u32) {}

        pub fn render(
            &self,
            _data: &[u8],
            _fw: u32,
            _fh: u32,
        ) -> Result<(), String> {
            Err("Display rendering is only supported on Windows".into())
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub::*;
