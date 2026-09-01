use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::event::LifecycleEventType;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
use windows::Win32::System::Shutdown::{
    ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy, ShutdownBlockReasonQuery,
};
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE};

static SHUTDOWN_HANDLING: AtomicBool = AtomicBool::new(false);

pub fn reset_shutdown_handling() {
    SHUTDOWN_HANDLING.store(false, Ordering::SeqCst);
}

pub fn begin_shutdown_handling() -> bool {
    !SHUTDOWN_HANDLING.swap(true, Ordering::SeqCst)
}

#[derive(Copy, Clone)]
pub struct SendHwnd(HWND);

unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

impl SendHwnd {
    pub fn new(hwnd: HWND) -> Self {
        Self(hwnd)
    }

    pub fn get(self) -> HWND {
        self.0
    }
}

pub struct WindowsGuard {
    hwnd: SendHwnd,
    event: LifecycleEventType,
    released: bool,
}

impl WindowsGuard {
    /// Delay mechanisms are acquired synchronously in the window procedure before
    /// the lifecycle event is queued; this guard only releases them after sync.
    pub fn new(hwnd: HWND, event: LifecycleEventType) -> LifecycleResult<Self> {
        Ok(Self {
            hwnd: SendHwnd::new(hwnd),
            event,
            released: false,
        })
    }
}

impl crate::lifecycle::platform::PlatformGuard for WindowsGuard {
    fn release(&mut self) -> LifecycleResult<()> {
        if self.released {
            return Ok(());
        }

        match self.event {
            LifecycleEventType::Shutdown | LifecycleEventType::Reboot => {
                unsafe {
                    let _ = ShutdownBlockReasonDestroy(self.hwnd.get());
                    let _ = ShowWindow(self.hwnd.get(), SW_HIDE);
                }
                reset_shutdown_handling();
            }
            LifecycleEventType::Sleep | LifecycleEventType::Hibernate => {
                unsafe {
                    SetThreadExecutionState(ES_CONTINUOUS);
                }
            }
        }

        self.released = true;
        Ok(())
    }
}

pub fn wide_string(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn block_shutdown(hwnd: HWND) -> LifecycleResult<()> {
    let reason = wide_string("Application synchronization in progress");
    unsafe {
        // Window must be visible when registering a shutdown block reason.
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        ShutdownBlockReasonCreate(hwnd, PCWSTR(reason.as_ptr()))
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;

        let mut buffer = vec![0u16; 256];
        let mut length = buffer.len() as u32;
        let queried = ShutdownBlockReasonQuery(
            hwnd,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        if queried.is_ok() && length > 0 {
            let end = length as usize;
            let text = String::from_utf16_lossy(&buffer[..end.min(buffer.len())])
                .trim_end_matches('\0')
                .to_string();
            crate::native_log::native_log(&format!(
                "native: ShutdownBlockReasonQuery confirmed reason={text:?}"
            ));
        }
    }
    Ok(())
}

pub fn hold_sleep_power() -> LifecycleResult<()> {
    unsafe {
        if SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED).0 == 0 {
            return Err(LifecycleError::Platform(
                "SetThreadExecutionState failed".into(),
            ));
        }
    }
    Ok(())
}
