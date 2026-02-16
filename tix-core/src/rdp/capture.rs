//! DXGI Desktop Duplication screen capture for Windows.
//!
//! Uses the Direct3D 11 Desktop Duplication API to obtain GPU-backed
//! screen frames with minimal latency (< 2 ms on modern hardware).
//!
//! # Platform
//!
//! This module is **Windows-only**. On other platforms the types are
//! still defined but construction will fail at runtime.

use std::time::Instant;

use crate::error::TixError;
use crate::rdp::types::{PixelFormat, RawScreenFrame};

// ── Platform gate ────────────────────────────────────────────────

/// DXGI-based screen capturer.
///
/// Wraps the `IDXGIOutputDuplication` pipeline:
///
/// 1. Create a D3D11 device.
/// 2. Enumerate outputs and duplicate the target monitor.
/// 3. Create a CPU-readable staging texture.
/// 4. On each call to [`capture_frame`](Self::capture_frame):
///    - `AcquireNextFrame` (blocks up to `timeout_ms`).
///    - Copy the desktop texture to the staging texture.
///    - Map, memcpy into a `Vec<u8>`, unmap, release.
///
/// # Safety
///
/// All unsafe FFI calls are confined to this struct.
pub struct DxgiCapturer {
    /// Screen width in pixels.
    width: u32,
    /// Screen height in pixels.
    height: u32,
    /// Row pitch of the staging texture.
    stride: u32,

    // ── Platform handles (Windows only) ──────────────────────
    #[cfg(target_os = "windows")]
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    #[cfg(target_os = "windows")]
    context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    #[cfg(target_os = "windows")]
    duplication: windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication,
    #[cfg(target_os = "windows")]
    staging_texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
}

// ── Windows implementation ───────────────────────────────────────

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use windows::{
        core::Interface,
        Win32::Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    };

    impl DxgiCapturer {
        /// Initialise the capturer for monitor `monitor_index` (0 = primary).
        pub fn new(monitor_index: u32) -> Result<Self, TixError> {
            unsafe { Self::init_dxgi(monitor_index) }
        }

        unsafe fn init_dxgi(monitor_index: u32) -> Result<Self, TixError> {
            // 1. Create D3D11 device + immediate context.
            let mut device = None;
            let mut context = None;
            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    None,
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None, // feature levels — let the driver decide
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .map_err(|e| TixError::Other(format!("D3D11CreateDevice failed: {e}")))?;
            }

            let device = device.ok_or_else(|| TixError::Other("D3D11 device is None".into()))?;
            let context =
                context.ok_or_else(|| TixError::Other("D3D11 context is None".into()))?;

            // 2. Traverse DXGI: Device → Adapter → Output.
            let dxgi_device: IDXGIDevice = device.cast().map_err(|e| {
                TixError::Other(format!("Cast to IDXGIDevice failed: {e}"))
            })?;
            let adapter = unsafe {
                dxgi_device
                    .GetAdapter()
                    .map_err(|e| TixError::Other(format!("GetAdapter failed: {e}")))?
            };
            let output: IDXGIOutput = unsafe {
                adapter
                    .EnumOutputs(monitor_index)
                    .map_err(|e| TixError::Other(format!("EnumOutputs({monitor_index}) failed: {e}")))?
            };

            // 3. Duplicate the output.
            let output1: IDXGIOutput1 = output.cast().map_err(|e| {
                TixError::Other(format!("Cast to IDXGIOutput1 failed: {e}"))
            })?;
            let duplication = unsafe {
                output1
                    .DuplicateOutput(&device)
                    .map_err(|e| TixError::Other(format!("DuplicateOutput failed: {e}")))?
            };

            // Get output dimensions from the duplication descriptor.
            let dup_desc = unsafe { duplication.GetDesc() };
            let width = dup_desc.ModeDesc.Width;
            let height = dup_desc.ModeDesc.Height;

            // 4. Create a CPU-readable staging texture.
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };

            let mut staging_texture = None;
            unsafe {
                device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging_texture))
                    .map_err(|e| TixError::Other(format!("CreateTexture2D (staging) failed: {e}")))?;
            }
            let staging_texture = staging_texture
                .ok_or_else(|| TixError::Other("Staging texture is None".into()))?;

            // Row pitch is unknown until we map; estimate 4 × width for now
            // (will be corrected on first capture).
            let stride = width * 4;

            Ok(Self {
                width,
                height,
                stride,
                device,
                context,
                duplication,
                staging_texture,
            })
        }

        /// Capture the next desktop frame.
        ///
        /// Blocks for up to `timeout_ms` milliseconds waiting for a new
        /// frame from the compositor. Returns [`TixError::Timeout`] if no
        /// new frame is available within the deadline.
        pub fn capture_frame(&mut self, timeout_ms: u32) -> Result<RawScreenFrame, TixError> {
            unsafe { self.capture_inner(timeout_ms) }
        }

        unsafe fn capture_inner(&mut self, timeout_ms: u32) -> Result<RawScreenFrame, TixError> {
            use windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAIT_TIMEOUT;

            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;

            match unsafe {
                self.duplication
                    .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
            } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    return Err(TixError::Timeout(std::time::Duration::from_millis(
                        timeout_ms as u64,
                    )));
                }
                Err(e) => {
                    return Err(TixError::Other(format!("AcquireNextFrame failed: {e}")));
                }
            }

            let resource =
                resource.ok_or_else(|| TixError::Other("Acquired resource is None".into()))?;

            let texture: ID3D11Texture2D = resource.cast().map_err(|e| {
                let _ = unsafe { self.duplication.ReleaseFrame() };
                TixError::Other(format!("Cast to ID3D11Texture2D failed: {e}"))
            })?;

            // Copy GPU texture → staging texture.
            unsafe {
                self.context
                    .CopyResource(&self.staging_texture, &texture);
            }

            // Release the DXGI frame as early as possible.
            let _ = unsafe { self.duplication.ReleaseFrame() };

            // Map the staging texture for CPU read.
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                self.context
                    .Map(
                        &self.staging_texture,
                        0,
                        D3D11_MAP_READ,
                        0,
                        Some(&mut mapped),
                    )
                    .map_err(|e| TixError::Other(format!("Map failed: {e}")))?;
            }

            let stride = mapped.RowPitch;
            let total_bytes = stride as usize * self.height as usize;
            let src = unsafe {
                std::slice::from_raw_parts(mapped.pData as *const u8, total_bytes)
            };
            let data = src.to_vec();

            unsafe { self.context.Unmap(&self.staging_texture, 0) };

            self.stride = stride;

            Ok(RawScreenFrame {
                width: self.width,
                height: self.height,
                stride,
                format: PixelFormat::Bgra8,
                data,
                timestamp: Instant::now(),
            })
        }

        /// Screen width in pixels.
        pub fn width(&self) -> u32 {
            self.width
        }

        /// Screen height in pixels.
        pub fn height(&self) -> u32 {
            self.height
        }
    }
}

// ── Non-Windows stub ─────────────────────────────────────────────

#[cfg(not(target_os = "windows"))]
impl DxgiCapturer {
    /// DXGI is only available on Windows.
    pub fn new(_monitor_index: u32) -> Result<Self, TixError> {
        Err(TixError::Other(
            "DXGI Desktop Duplication is only available on Windows".into(),
        ))
    }

    pub fn capture_frame(&mut self, _timeout_ms: u32) -> Result<RawScreenFrame, TixError> {
        Err(TixError::Other("Not supported on this platform".into()))
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}
