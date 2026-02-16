//! Windows service integration for tix-rdp-slave.
//!
//! Registers the process with the Windows Service Control Manager
//! (SCM) and translates service control messages (stop, shutdown)
//! into the `running` flag used by [`RdpSlaveService`].
//!
//! Also provides `install` / `uninstall` helpers.

#![cfg(target_os = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use tracing::{error, info};
use windows::core::{PCWSTR, w};
use windows::Win32::System::Services::*;

use crate::config::SlaveConfig;
use crate::service::RdpSlaveService;

// ── Globals (required by the SCM callback ABI) ──────────────────

/// Global stop flag shared with the SCM handler callback.
static STOP_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

const SERVICE_NAME: PCWSTR = w!("TixRdpSlave");
const SERVICE_DISPLAY: PCWSTR = w!("TIX RDP Slave Service");
const SERVICE_DESCRIPTION_TEXT: PCWSTR =
    w!("Ultra-fast remote desktop screen capture service for TIX");

// ── Public API ───────────────────────────────────────────────────

/// Run the process as a Windows service (called when launched by SCM).
pub fn run_as_windows_service(config: SlaveConfig) -> Result<(), Box<dyn std::error::Error>> {
    // We cannot use the Tokio runtime here yet — the SCM calls our
    // `service_main` entry and from there we launch the runtime.

    // The `service_main` callback receives the global config via a OnceLock.
    static CONFIG: OnceLock<SlaveConfig> = OnceLock::new();
    let _ = CONFIG.set(config);

    unsafe {
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: windows::core::PWSTR(SERVICE_NAME.as_ptr().cast_mut()),
                lpServiceProc: Some(service_main_trampoline),
            },
            SERVICE_TABLE_ENTRYW {
                lpServiceName: windows::core::PWSTR(std::ptr::null_mut()),
                lpServiceProc: None,
            },
        ];

        StartServiceCtrlDispatcherW(table.as_ptr())
            .map_err(|e| format!("StartServiceCtrlDispatcher failed: {e}"))?;
    }

    Ok(())
}

/// Install the service into the Windows SCM.
pub fn install_service() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let exe_path: Vec<u16> = exe
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CREATE_SERVICE)?;

        let result = CreateServiceW(
            scm,
            SERVICE_NAME,
            SERVICE_DISPLAY,
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            PCWSTR(exe_path.as_ptr()),
            None,
            None,
            None,
            None, // LocalSystem
            None,
        );

        match result {
            Ok(svc) => {
                info!("service installed");
                let desc = SERVICE_DESCRIPTIONW {
                    lpDescription: windows::core::PWSTR(SERVICE_DESCRIPTION_TEXT.as_ptr().cast_mut()),
                };
                let _ = ChangeServiceConfig2W(
                    svc,
                    SERVICE_CONFIG_DESCRIPTION,
                    Some(&desc as *const _ as *const std::ffi::c_void),
                );
                let _ = CloseServiceHandle(svc);
            }
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                return Err(format!("CreateService failed: {e}").into());
            }
        }

        let _ = CloseServiceHandle(scm);
    }

    Ok(())
}

/// Uninstall (remove) the service from the Windows SCM.
pub fn uninstall_service() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT)?;

        let svc = match OpenServiceW(scm, SERVICE_NAME, SERVICE_ALL_ACCESS) {
            Ok(h) => h,
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                return Err(format!("OpenService failed: {e}").into());
            }
        };

        // Try stopping first (ignore errors — may already be stopped).
        let mut status = SERVICE_STATUS::default();
        let _ = ControlService(svc, SERVICE_CONTROL_STOP, &mut status);

        DeleteService(svc).map_err(|e| format!("DeleteService failed: {e}"))?;
        info!("service uninstalled");

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
    }

    Ok(())
}

// ── SCM Callbacks ────────────────────────────────────────────────

/// Entry point called by the SCM.
unsafe extern "system" fn service_main_trampoline(
    _argc: u32,
    _argv: *mut windows::core::PWSTR,
) {
    // Register the control handler.
    let status_handle = match unsafe { RegisterServiceCtrlHandlerW(SERVICE_NAME, Some(ctrl_handler)) } {
        Ok(h) => h,
        Err(e) => {
            error!("RegisterServiceCtrlHandler failed: {e}");
            return;
        }
    };

    // Report START_PENDING.
    report_status(status_handle, SERVICE_START_PENDING, 0, 3000);

    // Build the service.
    let config = SlaveConfig::default(); // In production, load from file.
    let svc = RdpSlaveService::new(config);
    let _ = STOP_FLAG.set(svc.stop_handle());

    // Report RUNNING.
    report_status(status_handle, SERVICE_RUNNING, 0, 0);

    // Create a Tokio runtime and run.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        if let Err(e) = svc.run().await {
            error!("service error: {e}");
        }
    });

    // Report STOPPED.
    report_status(status_handle, SERVICE_STOPPED, 0, 0);
}

/// SCM control handler (stop, shutdown, etc.).
unsafe extern "system" fn ctrl_handler(control: u32) {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            if let Some(flag) = STOP_FLAG.get() {
                flag.store(false, Ordering::SeqCst);
            }
        }
        SERVICE_CONTROL_INTERROGATE => {
            // No-op — the SCM uses this to query status.
        }
        _ => {}
    }
}

/// Helper: set the service status with the SCM.
fn report_status(
    handle: SERVICE_STATUS_HANDLE,
    state: SERVICE_STATUS_CURRENT_STATE,
    exit_code: u32,
    wait_hint: u32,
) {
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            SERVICE_ACCEPT_STOP
        },
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: wait_hint,
    };
    unsafe {
        let _ = SetServiceStatus(handle, &status);
    }
}
