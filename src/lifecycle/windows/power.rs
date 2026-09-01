use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::dispatch::{emit_lifecycle_event, CallbackDispatcher};
use crate::lifecycle::event::{LifecycleEventType, PlatformEvent};
use crate::lifecycle::platform::{PlatformGuard, PlatformLifecycle};
use parking_lot::Mutex;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;
use tracing::{error, info, warn};
use windows::core::GUID;
use windows::Win32::Foundation::{GetLastError, BOOL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    RegisterPowerSettingNotification, RegisterSuspendResumeNotification,
    UnregisterPowerSettingNotification, UnregisterSuspendResumeNotification, HPOWERNOTIFY,
    POWERBROADCAST_SETTING,
};
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::System::SystemServices::GUID_LIDSWITCH_STATE_CHANGE;
use windows::Win32::System::Threading::SetProcessShutdownParameters;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, PostMessageW, RegisterClassW, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, DEVICE_NOTIFY_WINDOW_HANDLE, GWLP_USERDATA, MSG,
    SW_HIDE, WM_DESTROY, WM_ENDSESSION, WM_POWERBROADCAST, WM_QUERYENDSESSION, WM_USER,
    WM_WTSSESSION_CHANGE, WNDCLASSW, WS_EX_NOACTIVATE, WS_OVERLAPPEDWINDOW, WTS_SESSION_LOGOFF,
};

use super::native_log::{native_log, set_log_path};
use super::session::{
    begin_shutdown_handling, block_shutdown, hold_sleep_power, reset_shutdown_handling,
    wide_string, SendHwnd, WindowsGuard,
};
use crate::lifecycle::sync_wait::SyncWait;

const WM_LIFECYCLE_STOP: u32 = WM_USER + 1;
const WM_LIFECYCLE_SHUTDOWN: u32 = WM_USER + 2;
const PBT_APMSUSPEND: u32 = 0x0004;
const PBT_POWERSETTINGCHANGE: u32 = 0x8013;
const SHUTDOWN_PRIORITY_LAST: u32 = 0x4FF;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const ENDSESSION_CRITICAL: u32 = 0x4000_0000;

static WINDOW_CLASS: OnceLock<Vec<u16>> = OnceLock::new();
static LIFECYCLE_HWND: OnceLock<Mutex<SendHwnd>> = OnceLock::new();

fn lifecycle_hwnd_slot() -> &'static Mutex<SendHwnd> {
    LIFECYCLE_HWND.get_or_init(|| Mutex::new(SendHwnd::new(HWND::default())))
}

fn window_class_name() -> &'static [u16] {
    WINDOW_CLASS.get_or_init(|| wide_string("SystemLifecycleHiddenWindow"))
}

fn window_title() -> &'static [u16] {
    static TITLE: OnceLock<Vec<u16>> = OnceLock::new();
    TITLE.get_or_init(|| wide_string("System Lifecycle Sync"))
}

struct WindowState {
    hwnd: HWND,
    events: SyncSender<PlatformEvent>,
    dispatcher: Arc<CallbackDispatcher>,
    worker_ready: Receiver<()>,
    sync_wait: Arc<SyncWait>,
    max_wait: Duration,
}

impl WindowState {
    fn notify_shutdown(&self, event_type: LifecycleEventType, source: &str) -> bool {
        if !begin_shutdown_handling() {
            native_log(&format!(
                "native: duplicate shutdown ignored source={source} event={}",
                event_type.as_str()
            ));
            return true;
        }

        let line = format!(
            "native: shutdown detected source={source} event={}",
            event_type.as_str()
        );
        native_log(&line);
        info!(
            source,
            event = event_type.as_str(),
            "shutdown notification received"
        );
        if let Err(err) = block_shutdown(self.hwnd) {
            native_log(&format!("native: ShutdownBlockReasonCreate failed: {err}"));
            warn!(error = %err, source, "failed to create shutdown block reason");
            reset_shutdown_handling();
            return false;
        }
        native_log("native: ShutdownBlockReasonCreate succeeded");
        info!(source, "shutdown block reason created");
        emit_lifecycle_event(
            &self.events,
            &self.worker_ready,
            &self.dispatcher,
            event_type,
            source,
        );
        true
    }

    fn notify_sleep(&self, source: &str) {
        native_log(&format!("native: sleep detected source={source}"));
        info!(source, "sleep notification received");
        if let Err(err) = hold_sleep_power() {
            native_log(&format!("native: hold_sleep_power failed: {err}"));
            warn!(error = %err, source, "failed to hold sleep power state");
        } else {
            native_log("native: sleep power assertion created");
        }
        emit_lifecycle_event(
            &self.events,
            &self.worker_ready,
            &self.dispatcher,
            LifecycleEventType::Sleep,
            source,
        );
    }
}

fn post_shutdown_from_console(source: &'static str) {
    native_log(&format!(
        "native: {source} received, posting to window thread"
    ));
    let hwnd = lifecycle_hwnd_slot().lock().get();
    if hwnd.0.is_null() {
        native_log("native: console shutdown ignored, window unavailable");
        warn!(
            source,
            "console shutdown signal ignored; lifecycle window unavailable"
        );
        return;
    }
    unsafe {
        if let Err(err) = PostMessageW(hwnd, WM_LIFECYCLE_SHUTDOWN, WPARAM(0), LPARAM(0)) {
            native_log(&format!("native: PostMessageW failed: {err}"));
            warn!(error = %err, source, "failed to post shutdown message to lifecycle window");
        }
    }
}

unsafe extern "system" fn console_ctrl_handler(ctrl_type: u32) -> BOOL {
    match ctrl_type {
        CTRL_SHUTDOWN_EVENT => {
            post_shutdown_from_console("CTRL_SHUTDOWN_EVENT");
            BOOL(1)
        }
        CTRL_LOGOFF_EVENT => {
            post_shutdown_from_console("CTRL_LOGOFF_EVENT");
            BOOL(1)
        }
        _ => BOOL(0),
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;

    if msg == WM_LIFECYCLE_STOP {
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
        }
        return LRESULT(0);
    }

    if msg == WM_LIFECYCLE_SHUTDOWN {
        if !state_ptr.is_null() {
            let state = unsafe { &*state_ptr };
            let _ = state.notify_shutdown(LifecycleEventType::Shutdown, "console_ctrl_handler");
            native_log("native: WM_LIFECYCLE_SHUTDOWN returning allow=true");
            return LRESULT(1);
        }
        return LRESULT(1);
    }

    if state_ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    let state = unsafe { &*state_ptr };

    match msg {
        WM_QUERYENDSESSION => {
            let critical = (lparam.0 as u32) & ENDSESSION_CRITICAL != 0;
            if critical {
                native_log("native: critical shutdown, not blocking");
                let _ = state
                    .notify_shutdown(LifecycleEventType::Shutdown, "WM_QUERYENDSESSION_CRITICAL");
                return LRESULT(1);
            }
            let handled = state.notify_shutdown(LifecycleEventType::Shutdown, "WM_QUERYENDSESSION");
            native_log(&format!(
                "native: WM_QUERYENDSESSION returning allow=true handled={handled}"
            ));
            // TRUE keeps shutdown moving; WM_ENDSESSION holds until synccomplete.
            LRESULT(1)
        }
        WM_ENDSESSION => {
            let ending = wparam.0 != 0;
            native_log(&format!("native: WM_ENDSESSION ending={ending}"));
            info!(ending, "WM_ENDSESSION received");
            if ending {
                native_log("native: WM_ENDSESSION waiting for synccomplete");
                let completed = state.sync_wait.wait(state.max_wait);
                native_log(&format!(
                    "native: WM_ENDSESSION sync finished completed={completed}"
                ));
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                }
                native_log("native: PostQuitMessage sent after shutdown sync");
            }
            LRESULT(0)
        }
        WM_WTSSESSION_CHANGE => {
            let event = wparam.0 as u32;
            if event == WTS_SESSION_LOGOFF {
                let _ = state.notify_shutdown(LifecycleEventType::Shutdown, "WTS_SESSION_LOGOFF");
                native_log("native: WTS_SESSION_LOGOFF returning allow=true");
                return LRESULT(1);
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            let event_code = wparam.0 as u32;
            match event_code {
                PBT_APMSUSPEND => {
                    state.notify_sleep("PBT_APMSUSPEND");
                }
                PBT_POWERSETTINGCHANGE if lparam.0 != 0 => {
                    let setting = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
                    if setting.PowerSetting == GUID_LIDSWITCH_STATE_CHANGE
                        && setting.DataLength >= 4
                    {
                        let lid_open =
                            unsafe { (setting.Data.as_ptr() as *const u32).read_unaligned() };
                        if lid_open == 0 {
                            state.notify_sleep("GUID_LIDSWITCH_STATE_CHANGE");
                        }
                    }
                }
                _ => {}
            }
            LRESULT(1)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

struct NotificationHandles {
    suspend: HPOWERNOTIFY,
    lid: HPOWERNOTIFY,
}

impl NotificationHandles {
    unsafe fn unregister(self) {
        let _ = UnregisterSuspendResumeNotification(self.suspend);
        let _ = UnregisterPowerSettingNotification(self.lid);
    }
}

pub struct WindowsLifecycle {
    listener: Mutex<Option<JoinHandle<()>>>,
    hwnd: Arc<Mutex<SendHwnd>>,
}

impl WindowsLifecycle {
    pub fn new() -> LifecycleResult<Self> {
        Ok(Self {
            listener: Mutex::new(None),
            hwnd: Arc::new(Mutex::new(SendHwnd::new(HWND::default()))),
        })
    }
}

impl PlatformLifecycle for WindowsLifecycle {
    fn start(
        &mut self,
        events: SyncSender<PlatformEvent>,
        dispatcher: Arc<CallbackDispatcher>,
        worker_ready: Receiver<()>,
        sync_wait: Arc<SyncWait>,
        max_wait: Duration,
    ) -> LifecycleResult<()> {
        if let Ok(cwd) = std::env::current_dir() {
            set_log_path(cwd.join("lifecycle_native.log"));
        }
        native_log("native: lifecycle listener starting");
        reset_shutdown_handling();

        let hwnd_slot = Arc::clone(&self.hwnd);
        let handle = std::thread::Builder::new()
            .name("windows-lifecycle-listener".into())
            .spawn(move || {
                if let Err(err) = run_message_loop(
                    events,
                    dispatcher,
                    worker_ready,
                    sync_wait,
                    max_wait,
                    hwnd_slot,
                ) {
                    native_log(&format!("native: message loop failed: {err}"));
                    error!(error = %err, "windows lifecycle listener failed");
                }
            })
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;
        *self.listener.lock() = Some(handle);
        Ok(())
    }

    fn stop(&mut self) -> LifecycleResult<()> {
        let hwnd = self.hwnd.lock().get();
        if !hwnd.0.is_null() {
            unsafe {
                let _ = PostMessageW(hwnd, WM_LIFECYCLE_STOP, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(handle) = self.listener.lock().take() {
            let _ = handle.join();
        }
        *self.hwnd.lock() = SendHwnd::new(HWND::default());
        *lifecycle_hwnd_slot().lock() = SendHwnd::new(HWND::default());
        unsafe {
            let _ = SetConsoleCtrlHandler(None, false);
        }
        native_log("native: lifecycle listener stopped");
        Ok(())
    }

    fn acquire_guard(&self, event: LifecycleEventType) -> LifecycleResult<Box<dyn PlatformGuard>> {
        let hwnd = self.hwnd.lock().get();
        if hwnd.0.is_null() {
            return Err(LifecycleError::Platform(
                "windows lifecycle window unavailable".into(),
            ));
        }
        Ok(Box::new(WindowsGuard::new(hwnd, event)?))
    }
}

fn run_message_loop(
    events: SyncSender<PlatformEvent>,
    dispatcher: Arc<CallbackDispatcher>,
    worker_ready: Receiver<()>,
    sync_wait: Arc<SyncWait>,
    max_wait: Duration,
    hwnd_slot: Arc<Mutex<SendHwnd>>,
) -> LifecycleResult<()> {
    unsafe {
        if let Err(err) = SetProcessShutdownParameters(SHUTDOWN_PRIORITY_LAST, 0) {
            warn!(error = %err, "SetProcessShutdownParameters failed");
        }

        let class_name = window_class_name();
        let title = window_title();
        let hinstance =
            GetModuleHandleW(None).map_err(|err| LifecycleError::Platform(err.to_string()))?;

        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: hinstance.into(),
            lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
            style: CS_HREDRAW | CS_VREDRAW,
            ..Default::default()
        };
        let atom = RegisterClassW(&class);
        if atom == 0 {
            let last_error = GetLastError().0;
            if last_error != ERROR_CLASS_ALREADY_EXISTS {
                return Err(LifecycleError::Platform(format!(
                    "RegisterClassW failed with error {last_error}"
                )));
            }
        }

        // Top-level window; kept off-screen until shutdown blocking needs it visible.
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE,
            windows::core::PCWSTR(class_name.as_ptr()),
            windows::core::PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            -2000,
            -2000,
            320,
            120,
            None,
            None,
            hinstance,
            None,
        )
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

        let _ = ShowWindow(hwnd, SW_HIDE);
        *hwnd_slot.lock() = SendHwnd::new(hwnd);
        *lifecycle_hwnd_slot().lock() = SendHwnd::new(hwnd);

        if GetConsoleWindow().0 != std::ptr::null_mut() {
            SetConsoleCtrlHandler(Some(console_ctrl_handler), true)
                .map_err(|err| LifecycleError::Platform(err.to_string()))?;
            native_log("native: SetConsoleCtrlHandler registered");
        } else {
            native_log("native: no console window; skipping SetConsoleCtrlHandler");
        }

        let state = Box::new(WindowState {
            hwnd,
            events,
            dispatcher,
            worker_ready,
            sync_wait,
            max_wait,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

        let suspend = RegisterSuspendResumeNotification(hwnd, DEVICE_NOTIFY_WINDOW_HANDLE)
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;
        let lid = RegisterPowerSettingNotification(
            hwnd,
            &GUID_LIDSWITCH_STATE_CHANGE as *const GUID,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;
        let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);

        let notifications = NotificationHandles { suspend, lid };

        native_log("native: lifecycle window created (hidden until shutdown block)");
        info!("windows lifecycle window created");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = WTSUnRegisterSessionNotification(hwnd);
        notifications.unregister();
        let _ = SetConsoleCtrlHandler(None, false);
        *lifecycle_hwnd_slot().lock() = SendHwnd::new(HWND::default());

        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
        if !state_ptr.is_null() {
            drop(Box::from_raw(state_ptr));
        }
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = DestroyWindow(hwnd);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_constants_match_win32_headers() {
        assert_eq!(PBT_APMSUSPEND, 4);
        assert_eq!(PBT_POWERSETTINGCHANGE, 0x8013);
    }
}
