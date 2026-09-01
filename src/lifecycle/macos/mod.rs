mod power;

use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::dispatch::{emit_lifecycle_event, CallbackDispatcher};
use crate::lifecycle::event::{LifecycleEventType, PlatformEvent};
use crate::lifecycle::platform::{PlatformGuard, PlatformLifecycle};
use crate::lifecycle::sync_wait::SyncWait;
use crate::native_log::native_log;
use core_foundation::base::TCFType;
use core_foundation::runloop::{
    kCFRunLoopCommonModes, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRun,
    CFRunLoopStop, CFRunLoopWakeUp,
};
use core_foundation::string::CFString;
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::notification_center::{
    CFNotificationCenterAddObserver, CFNotificationCenterGetDarwinNotifyCenter,
    CFNotificationCenterGetDistributedCenter, CFNotificationCenterRef,
    CFNotificationCenterRemoveEveryObserver, CFNotificationName,
    CFNotificationSuspensionBehaviorDeliverImmediately,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, Sel};
use objc2::{class, msg_send, sel, ClassType};
use parking_lot::Mutex;
use power::{
    allow_power_change, begin_shutdown_handling, begin_sleep_handling, hold_sleep_power,
    reset_shutdown_handling, reset_sleep_handling, IoConnect, IoNotificationPortRef, IoObject,
    IoService, MacOsGuard, K_CLAMSHELL_STATE_BIT, K_IOPM_MESSAGE_CLAMSHELL_STATE_CHANGE,
    K_IOPM_ROOT_DOMAIN, K_IO_GENERAL_INTEREST, K_IO_MESSAGE_CAN_SYSTEM_SLEEP,
    K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON, K_IO_MESSAGE_SYSTEM_WILL_NOT_SLEEP,
    K_IO_MESSAGE_SYSTEM_WILL_SLEEP,
};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;
use tracing::{error, info, warn};

const NS_TERMINATE_NOW: usize = 1;
const NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY: i64 = 1;
const K_CURRENT_PROCESS: u32 = 2;
const K_PROCESS_TRANSFORM_TO_UI_ELEMENT_APPLICATION: u32 = 4;
const K_AE_QUIT_REASON: u32 = 0x7768_793F; // 'why?'
const K_AE_SHUTDOWN: u32 = 0x7368_7574; // 'shut'
const K_AE_RESTART: u32 = 0x7265_7374; // 'rest'
const K_CORE_EVENT_CLASS: u32 = 0x6165_7674; // 'aevt'
const K_AE_QUIT_APPLICATION: u32 = 0x7175_6974; // 'quit'

/// loginwindow posts these when the user confirms Shut Down / Restart / Log Out.
/// Unlike NSWorkspaceWillPowerOff, these are delivered on this thread's CFRunLoop.
const SHUTDOWN_NOTIFY_NAMES: &[&str] = &[
    "com.apple.system.loginwindow.shutdownInitiated",
    "com.apple.system.loginwindow.restartInitiated",
    "com.apple.system.loginwindow.restartinitiated",
    "com.apple.system.loginwindow.shutdownNow",
    "com.apple.system.loginwindow.logoutNow",
    "com.apple.shutdownInitiated",
    "com.apple.restartInitiated",
    "com.apple.logoutInitiated",
    "NSWorkspaceWillPowerOffNotification",
];

static SHUTDOWN_SIGNAL: AtomicBool = AtomicBool::new(false);

struct SendRunLoop(CFRunLoopRef);
unsafe impl Send for SendRunLoop {}
unsafe impl Sync for SendRunLoop {}

static LISTENER_STATE: OnceLock<Mutex<Option<Arc<MacListenerState>>>> = OnceLock::new();
static RUN_LOOP: Mutex<Option<SendRunLoop>> = Mutex::new(None);

fn listener_slot() -> &'static Mutex<Option<Arc<MacListenerState>>> {
    LISTENER_STATE.get_or_init(|| Mutex::new(None))
}

struct MacListenerState {
    events: SyncSender<PlatformEvent>,
    dispatcher: Arc<CallbackDispatcher>,
    worker_ready: std::sync::Mutex<Receiver<()>>,
    sync_wait: Arc<SyncWait>,
    max_wait: Duration,
    power_connection: std::sync::Mutex<IoConnect>,
}

unsafe impl Send for MacListenerState {}
unsafe impl Sync for MacListenerState {}

impl MacListenerState {
    fn with_worker_ready<R>(&self, f: impl FnOnce(&Receiver<()>) -> R) -> R {
        let ready = self
            .worker_ready
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        f(&ready)
    }

    fn notify_sleep(&self, source: &str) -> bool {
        if !begin_sleep_handling() {
            native_log(&format!("native: duplicate sleep ignored source={source}"));
            return false;
        }

        native_log(&format!("native: sleep detected source={source}"));
        info!(source, "sleep / lid notification received");
        if let Err(err) = hold_sleep_power() {
            native_log(&format!("native: hold_sleep_power failed: {err}"));
            warn!(error = %err, source, "failed to create macOS sleep assertion");
            reset_sleep_handling();
            return false;
        }

        self.with_worker_ready(|worker_ready| {
            emit_lifecycle_event(
                &self.events,
                worker_ready,
                &self.dispatcher,
                LifecycleEventType::Sleep,
                source,
            );
        });
        true
    }

    fn notify_shutdown(&self, source: &str, event_type: LifecycleEventType) -> bool {
        if !begin_shutdown_handling() {
            native_log(&format!(
                "native: duplicate shutdown ignored source={source} event={}",
                event_type.as_str()
            ));
            return false;
        }

        native_log(&format!(
            "native: shutdown detected source={source} event={}",
            event_type.as_str()
        ));
        info!(
            source,
            event = event_type.as_str(),
            "shutdown notification received"
        );

        self.with_worker_ready(|worker_ready| {
            emit_lifecycle_event(
                &self.events,
                worker_ready,
                &self.dispatcher,
                event_type,
                source,
            );
        });
        true
    }

    fn wait_for_sync(&self, source: &str) -> bool {
        native_log(&format!("native: {source} waiting for synccomplete"));
        let completed = self.sync_wait.wait(self.max_wait);
        native_log(&format!(
            "native: {source} sync finished completed={completed}"
        ));
        completed
    }
}

pub struct MacOsLifecycle {
    listener: Mutex<Option<JoinHandle<()>>>,
    stop_flag: Arc<AtomicBool>,
}

impl MacOsLifecycle {
    pub fn new() -> LifecycleResult<Self> {
        Ok(Self {
            listener: Mutex::new(None),
            stop_flag: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl PlatformLifecycle for MacOsLifecycle {
    fn start(
        &mut self,
        events: SyncSender<PlatformEvent>,
        dispatcher: Arc<CallbackDispatcher>,
        worker_ready: Receiver<()>,
        sync_wait: Arc<SyncWait>,
        max_wait: Duration,
    ) -> LifecycleResult<()> {
        self.stop_flag.store(false, Ordering::SeqCst);
        reset_shutdown_handling();
        reset_sleep_handling();

        let stop_flag = Arc::clone(&self.stop_flag);
        let handle = std::thread::Builder::new()
            .name("macos-lifecycle-listener".into())
            .spawn(move || {
                if let Err(err) = run_macos_listener(
                    events,
                    dispatcher,
                    worker_ready,
                    sync_wait,
                    max_wait,
                    stop_flag,
                ) {
                    native_log(&format!("native: macos listener failed: {err}"));
                    error!(error = %err, "macos lifecycle listener failed");
                }
            })
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;

        *self.listener.lock() = Some(handle);
        Ok(())
    }

    fn stop(&mut self) -> LifecycleResult<()> {
        self.stop_flag.store(true, Ordering::SeqCst);
        SHUTDOWN_SIGNAL.store(false, Ordering::SeqCst);
        if let Some(run_loop) = RUN_LOOP.lock().as_ref().map(|rl| rl.0) {
            unsafe {
                CFRunLoopStop(run_loop);
                CFRunLoopWakeUp(run_loop);
            }
        }
        stop_ns_application();
        if let Some(handle) = self.listener.lock().take() {
            let _ = handle.join();
        }
        *listener_slot().lock() = None;
        native_log("native: macos lifecycle listener stopped");
        Ok(())
    }

    fn acquire_guard(&self, event: LifecycleEventType) -> LifecycleResult<Box<dyn PlatformGuard>> {
        Ok(Box::new(MacOsGuard::new(event)?))
    }
}

fn run_macos_listener(
    events: SyncSender<PlatformEvent>,
    dispatcher: Arc<CallbackDispatcher>,
    worker_ready: Receiver<()>,
    sync_wait: Arc<SyncWait>,
    max_wait: Duration,
    stop_flag: Arc<AtomicBool>,
) -> LifecycleResult<()> {
    native_log("native: macos lifecycle listener starting");

    let state = Arc::new(MacListenerState {
        events,
        dispatcher,
        worker_ready: std::sync::Mutex::new(worker_ready),
        sync_wait,
        max_wait,
        power_connection: std::sync::Mutex::new(0),
    });
    *listener_slot().lock() = Some(Arc::clone(&state));
    disable_sudden_termination();
    transform_to_uielement_app();
    install_unix_shutdown_signals();

    let mut power_session = match register_iokit_power(&state) {
        Ok(session) => Some(session),
        Err(err) => {
            native_log(&format!(
                "native: IOKit power registration failed (sleep/lid delay limited): {err}"
            ));
            warn!(error = %err, "IOKit power registration failed");
            None
        }
    };

    let app_delegate = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        configure_application_delegate,
    )) {
        Ok(Ok(delegate)) => {
            register_workspace_observers(&delegate);
            install_quit_apple_event_handler(&delegate);
            Some(delegate)
        }
        Ok(Err(err)) => {
            native_log(&format!(
                "native: NSApplication setup failed (shutdown hold limited): {err}"
            ));
            warn!(error = %err, "NSApplication setup failed");
            None
        }
        Err(_) => {
            native_log("native: NSApplication setup panicked; continuing with IOKit only");
            None
        }
    };
    native_log("native: macos IOKit + NSWorkspace listeners registered");
    info!("macos lifecycle notifications registered (shutdown + lid/sleep)");

    let run_loop = unsafe { CFRunLoopGetCurrent() };
    *RUN_LOOP.lock() = Some(SendRunLoop(run_loop));

    let notify_observer = Arc::as_ptr(&state) as *const c_void;
    register_shutdown_notifications(notify_observer);
    spawn_signal_watcher(Arc::clone(&stop_flag));

    native_log("native: entering run loop (IOKit sleep + AppKit/Darwin shutdown)");
    while !stop_flag.load(Ordering::SeqCst) {
        unsafe {
            CFRunLoopRun();
        }
        if stop_flag.load(Ordering::SeqCst) {
            break;
        }
        if SHUTDOWN_SIGNAL.swap(false, Ordering::SeqCst) {
            handle_shutdown_and_wait("SIGTERM");
        }
    }

    unregister_shutdown_notifications(notify_observer);
    if let Some(session) = power_session.as_mut() {
        session.unregister();
    }
    if let Some(delegate) = app_delegate.as_ref() {
        unregister_workspace_observers(delegate);
    }
    *RUN_LOOP.lock() = None;
    *listener_slot().lock() = None;
    Ok(())
}

fn current_state() -> Option<Arc<MacListenerState>> {
    listener_slot().lock().clone()
}

fn handle_shutdown_notification(source: &str) {
    native_log(&format!("native: shutdown source={source}"));
    let event_type = if source.to_ascii_lowercase().contains("restart") {
        LifecycleEventType::Reboot
    } else {
        apple_event_lifecycle_type()
    };
    if let Some(state) = current_state() {
        let _ = state.notify_shutdown(source, event_type);
    }
}

/// Emit the Python event, then block until `synccomplete()` or timeout.
///
/// Only call this from a path that actually delays the OS (AppKit terminate,
/// quit Apple Event, or SIGTERM after we have become a UIElement app). Darwin
/// notify observers must not wait — loginwindow does not wait for them.
fn handle_shutdown_and_wait(source: &str) {
    handle_shutdown_notification(source);
    if let Some(state) = current_state() {
        let _ = state.wait_for_sync(source);
    }
}

#[repr(C)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn TransformProcessType(psn: *mut ProcessSerialNumber, transform_state: u32) -> i32;
}

/// Make this CLI process a UIElement app so loginwindow sends
/// `applicationShouldTerminate` / `kAEQuitApplication` and waits, instead of
/// SIGTERM followed quickly by SIGKILL.
fn transform_to_uielement_app() {
    let mut psn = ProcessSerialNumber {
        high_long_of_psn: 0,
        low_long_of_psn: K_CURRENT_PROCESS,
    };
    let status =
        unsafe { TransformProcessType(&mut psn, K_PROCESS_TRANSFORM_TO_UI_ELEMENT_APPLICATION) };
    native_log(&format!(
        "native: TransformProcessType UIElement status={status}"
    ));
}

fn disable_sudden_termination() {
    let info: *mut AnyObject = unsafe { msg_send![class!(NSProcessInfo), processInfo] };
    if info.is_null() {
        return;
    }
    let _: () = unsafe { msg_send![info, disableSuddenTermination] };
    let reason: *mut AnyObject = unsafe {
        msg_send![class!(NSString), stringWithUTF8String: b"system-lifecycle-extension sync\0".as_ptr()]
    };
    let _: () = unsafe { msg_send![info, disableAutomaticTermination: reason] };
    native_log("native: sudden/automatic termination disabled");
}

fn stop_ns_application() {
    let app: *mut AnyObject = unsafe { msg_send![class!(NSApplication), sharedApplication] };
    if !app.is_null() {
        let _: () = unsafe { msg_send![app, stop: std::ptr::null_mut::<AnyObject>()] };
    }
}

extern "C" fn unix_shutdown_signal(_sig: libc::c_int) {
    SHUTDOWN_SIGNAL.store(true, Ordering::SeqCst);
}

fn install_unix_shutdown_signals() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            unix_shutdown_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            unix_shutdown_signal as *const () as libc::sighandler_t,
        );
    }
    native_log("native: SIGTERM/SIGHUP handlers installed");
}

fn spawn_signal_watcher(stop_flag: Arc<AtomicBool>) {
    let _ = std::thread::Builder::new()
        .name("macos-shutdown-signal".into())
        .spawn(move || {
            let notify_tokens = register_loginwindow_notify_tokens();
            while !stop_flag.load(Ordering::SeqCst) {
                if SHUTDOWN_SIGNAL.swap(false, Ordering::SeqCst) {
                    handle_shutdown_and_wait("SIGTERM");
                    break;
                }
                if let Some(name) = poll_loginwindow_notify(&notify_tokens) {
                    handle_shutdown_and_wait(name);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
}

const LOGINWINDOW_NOTIFY_NAMES: &[&str] = &[
    "com.apple.system.loginwindow.shutdownInitiated",
    "com.apple.system.loginwindow.restartInitiated",
    "com.apple.system.loginwindow.logoutNow",
];

extern "C" {
    fn notify_register_check(name: *const libc::c_char, out_token: *mut i32) -> u32;
    fn notify_check(token: i32, check: *mut i32) -> u32;
}

fn register_loginwindow_notify_tokens() -> Vec<(i32, &'static str)> {
    let mut tokens = Vec::new();
    for name in LOGINWINDOW_NOTIFY_NAMES {
        let c_name = std::ffi::CString::new(*name).expect("notify name");
        let mut token = 0i32;
        let status = unsafe { notify_register_check(c_name.as_ptr(), &mut token) };
        if status != 0 {
            native_log(&format!(
                "native: notify_register_check failed name={name} status={status}"
            ));
            continue;
        }
        // First notify_check is always true; consume the initial state.
        let mut check = 0i32;
        unsafe {
            notify_check(token, &mut check);
        }
        tokens.push((token, *name));
    }
    if !tokens.is_empty() {
        native_log("native: loginwindow notify(3) tokens registered");
    }
    tokens
}

fn poll_loginwindow_notify(tokens: &[(i32, &'static str)]) -> Option<&'static str> {
    for (token, name) in tokens {
        let mut check = 0i32;
        let status = unsafe { notify_check(*token, &mut check) };
        if status == 0 && check != 0 {
            native_log(&format!("native: notify(3) fired name={name}"));
            return Some(*name);
        }
    }
    None
}

fn register_shutdown_notifications(observer: *const c_void) {
    unsafe {
        add_notify_names(CFNotificationCenterGetDarwinNotifyCenter(), observer);
        add_notify_names(CFNotificationCenterGetDistributedCenter(), observer);
    }
    native_log("native: Darwin + distributed shutdown notifications registered");
}

fn add_notify_names(center: CFNotificationCenterRef, observer: *const c_void) {
    for name in SHUTDOWN_NOTIFY_NAMES {
        let cf = CFString::new(name);
        unsafe {
            CFNotificationCenterAddObserver(
                center,
                observer,
                shutdown_notify_callback,
                cf.as_concrete_TypeRef(),
                std::ptr::null(),
                CFNotificationSuspensionBehaviorDeliverImmediately,
            );
        }
    }
}

fn unregister_shutdown_notifications(observer: *const c_void) {
    unsafe {
        CFNotificationCenterRemoveEveryObserver(
            CFNotificationCenterGetDarwinNotifyCenter(),
            observer,
        );
        CFNotificationCenterRemoveEveryObserver(
            CFNotificationCenterGetDistributedCenter(),
            observer,
        );
    }
}

extern "C" fn shutdown_notify_callback(
    _center: CFNotificationCenterRef,
    _observer: *mut c_void,
    name: CFNotificationName,
    _object: *const c_void,
    _user_info: CFDictionaryRef,
) {
    if name.is_null() {
        handle_shutdown_notification("cf-notify");
        return;
    }
    let cf = unsafe { CFString::wrap_under_get_rule(name) };
    handle_shutdown_notification(&cf.to_string());
}

fn install_quit_apple_event_handler(delegate: &NSObject) {
    let manager: *mut AnyObject =
        unsafe { msg_send![class!(NSAppleEventManager), sharedAppleEventManager] };
    if manager.is_null() {
        native_log("native: NSAppleEventManager unavailable");
        return;
    }
    let _: () = unsafe {
        msg_send![
            manager,
            setEventHandler: delegate,
            andSelector: sel!(handleQuitEvent:withReplyEvent:),
            forEventClass: K_CORE_EVENT_CLASS,
            andEventID: K_AE_QUIT_APPLICATION
        ]
    };
    native_log("native: kAEQuitApplication handler installed");
}

unsafe extern "C" fn handle_quit_event(
    _this: &NSObject,
    _cmd: Sel,
    _event: *mut AnyObject,
    _reply: *mut AnyObject,
) {
    native_log("native: kAEQuitApplication");
    handle_shutdown_and_wait("kAEQuitApplication");
}

fn configure_application_delegate() -> LifecycleResult<Retained<NSObject>> {
    let app: *mut AnyObject = unsafe { msg_send![class!(NSApplication), sharedApplication] };
    if app.is_null() {
        return Err(LifecycleError::Platform(
            "NSApplication.sharedApplication returned nil".into(),
        ));
    }

    transform_to_uielement_app();

    let accepted: bool = unsafe {
        msg_send![
            app,
            setActivationPolicy: NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY
        ]
    };
    native_log(&format!(
        "native: setActivationPolicy accessory accepted={accepted}"
    ));

    let delegate_class = terminate_delegate_class();
    let delegate: *mut AnyObject = unsafe { msg_send![delegate_class, new] };
    if delegate.is_null() {
        return Err(LifecycleError::Platform(
            "failed to allocate macOS terminate delegate".into(),
        ));
    }

    let existing: *mut AnyObject = unsafe { msg_send![app, delegate] };
    if existing.is_null() {
        let _: () = unsafe { msg_send![app, setDelegate: delegate] };
        native_log("native: NSApplication accessory delegate installed");
    } else {
        native_log(
            "native: existing NSApplication delegate left in place; using NSWorkspace + IOKit",
        );
    }

    let _: () = unsafe { msg_send![app, finishLaunching] };

    // SAFETY: `new` returned a +1 retain.
    Ok(unsafe { Retained::from_raw(delegate as *mut NSObject).unwrap() })
}

fn terminate_delegate_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        if let Some(existing) = AnyClass::get("SLETerminateDelegate") {
            return existing;
        }
        let mut builder =
            objc2::runtime::ClassBuilder::new("SLETerminateDelegate", NSObject::class())
                .expect("allocate SLETerminateDelegate");
        unsafe {
            builder.add_method(
                sel!(applicationShouldTerminate:),
                application_should_terminate as unsafe extern "C" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(workspaceWillSleep:),
                workspace_will_sleep as unsafe extern "C" fn(_, _, _),
            );
            builder.add_method(
                sel!(workspaceWillPowerOff:),
                workspace_will_power_off as unsafe extern "C" fn(_, _, _),
            );
            builder.add_method(
                sel!(handleQuitEvent:withReplyEvent:),
                handle_quit_event as unsafe extern "C" fn(_, _, _, _),
            );
        }
        builder.register()
    })
}

fn register_workspace_observers(delegate: &NSObject) {
    let workspace: *mut AnyObject = unsafe { msg_send![class!(NSWorkspace), sharedWorkspace] };
    if workspace.is_null() {
        native_log("native: NSWorkspace.sharedWorkspace unavailable");
        return;
    }
    let center: *mut AnyObject = unsafe { msg_send![workspace, notificationCenter] };
    if center.is_null() {
        native_log("native: NSWorkspace notificationCenter unavailable");
        return;
    }
    let sleep_name: *mut AnyObject = unsafe {
        msg_send![class!(NSString), stringWithUTF8String: b"NSWorkspaceWillSleepNotification\0".as_ptr()]
    };
    let power_name: *mut AnyObject = unsafe {
        msg_send![
            class!(NSString),
            stringWithUTF8String: b"NSWorkspaceWillPowerOffNotification\0".as_ptr()
        ]
    };
    unsafe {
        let _: () = msg_send![
            center,
            addObserver: delegate,
            selector: sel!(workspaceWillSleep:),
            name: sleep_name,
            object: std::ptr::null_mut::<AnyObject>()
        ];
        let _: () = msg_send![
            center,
            addObserver: delegate,
            selector: sel!(workspaceWillPowerOff:),
            name: power_name,
            object: std::ptr::null_mut::<AnyObject>()
        ];
    }
    native_log("native: NSWorkspace sleep/power-off observers registered");
}

fn unregister_workspace_observers(delegate: &NSObject) {
    let workspace: *mut AnyObject = unsafe { msg_send![class!(NSWorkspace), sharedWorkspace] };
    if workspace.is_null() {
        return;
    }
    let center: *mut AnyObject = unsafe { msg_send![workspace, notificationCenter] };
    if center.is_null() {
        return;
    }
    let _: () = unsafe { msg_send![center, removeObserver: delegate] };
}

unsafe extern "C" fn workspace_will_sleep(
    _this: &NSObject,
    _cmd: Sel,
    _notification: *mut AnyObject,
) {
    native_log("native: NSWorkspaceWillSleepNotification");
    if let Some(state) = current_state() {
        let _ = state.notify_sleep("NSWorkspaceWillSleepNotification");
    }
}

unsafe extern "C" fn workspace_will_power_off(
    _this: &NSObject,
    _cmd: Sel,
    _notification: *mut AnyObject,
) {
    native_log("native: NSWorkspaceWillPowerOffNotification");
    handle_shutdown_notification("NSWorkspaceWillPowerOffNotification");
}

unsafe extern "C" fn application_should_terminate(
    _this: &NSObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
) -> usize {
    native_log("native: applicationShouldTerminate received");
    handle_shutdown_and_wait("applicationShouldTerminate");
    native_log("native: applicationShouldTerminate returning NSTerminateNow");
    NS_TERMINATE_NOW
}

fn apple_event_lifecycle_type() -> LifecycleEventType {
    let manager: *mut AnyObject =
        unsafe { msg_send![class!(NSAppleEventManager), sharedAppleEventManager] };
    if manager.is_null() {
        return LifecycleEventType::Shutdown;
    }
    let event: *mut AnyObject = unsafe { msg_send![manager, currentAppleEvent] };
    if event.is_null() {
        return LifecycleEventType::Shutdown;
    }
    let reason: *mut AnyObject =
        unsafe { msg_send![event, attributeDescriptorForKeyword: K_AE_QUIT_REASON] };
    if reason.is_null() {
        return LifecycleEventType::Shutdown;
    }
    let code: u32 = unsafe { msg_send![reason, typeCodeValue] };
    native_log(&format!("native: Apple Event quit reason=0x{code:08x}"));
    if code == K_AE_RESTART {
        LifecycleEventType::Reboot
    } else if code == K_AE_SHUTDOWN {
        LifecycleEventType::Shutdown
    } else {
        LifecycleEventType::Shutdown
    }
}

struct PowerSession {
    notify_port: IoNotificationPortRef,
    power_notifier: IoObject,
    clamshell_notifier: IoObject,
    root_domain: IoService,
    connection: IoConnect,
}

impl PowerSession {
    fn unregister(&mut self) {
        unsafe {
            if self.power_notifier != 0 {
                let mut notifier = self.power_notifier;
                let _ = power::IODeregisterForSystemPower(&mut notifier);
                self.power_notifier = 0;
            }
            if self.clamshell_notifier != 0 {
                let _ = power::IOObjectRelease(self.clamshell_notifier);
                self.clamshell_notifier = 0;
            }
            if self.root_domain != 0 {
                let _ = power::IOObjectRelease(self.root_domain);
                self.root_domain = 0;
            }
            if !self.notify_port.is_null() {
                power::IONotificationPortDestroy(self.notify_port);
                self.notify_port = std::ptr::null_mut();
            }
            if self.connection != 0 {
                let _ = power::IOServiceClose(self.connection);
                self.connection = 0;
            }
        }
    }
}

impl Drop for PowerSession {
    fn drop(&mut self) {
        self.unregister();
    }
}

fn register_iokit_power(state: &Arc<MacListenerState>) -> LifecycleResult<PowerSession> {
    let mut notify_port: IoNotificationPortRef = std::ptr::null_mut();
    let mut power_notifier: IoObject = 0;
    let refcon = Arc::as_ptr(state) as *mut c_void;

    let connection = unsafe {
        power::IORegisterForSystemPower(
            refcon,
            &mut notify_port,
            iokit_power_callback,
            &mut power_notifier,
        )
    };
    if connection == 0 || notify_port.is_null() {
        return Err(LifecycleError::Platform(
            "IORegisterForSystemPower failed".into(),
        ));
    }
    *state
        .power_connection
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = connection;

    unsafe {
        let source = power::IONotificationPortGetRunLoopSource(notify_port);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
    }

    let matching = unsafe { power::IOServiceMatching(K_IOPM_ROOT_DOMAIN.as_ptr() as *const i8) };
    let root_domain =
        unsafe { power::IOServiceGetMatchingService(power::K_IO_MAIN_PORT_DEFAULT, matching) };
    let mut clamshell_notifier: IoObject = 0;
    if root_domain != 0 {
        let result = unsafe {
            power::IOServiceAddInterestNotification(
                notify_port,
                root_domain,
                K_IO_GENERAL_INTEREST.as_ptr() as *const i8,
                iokit_power_callback,
                refcon,
                &mut clamshell_notifier,
            )
        };
        if result != power::K_IO_RETURN_SUCCESS {
            native_log(&format!(
                "native: clamshell interest registration failed code={result}"
            ));
        } else {
            native_log("native: clamshell (lid) interest registered");
        }
    } else {
        native_log("native: IOPMrootDomain unavailable; lid close uses WillSleep only");
    }

    native_log("native: IORegisterForSystemPower succeeded");
    Ok(PowerSession {
        notify_port,
        power_notifier,
        clamshell_notifier,
        root_domain,
        connection,
    })
}

unsafe extern "C" fn iokit_power_callback(
    refcon: *mut c_void,
    _service: IoService,
    message_type: u32,
    message_argument: *mut c_void,
) {
    if refcon.is_null() {
        return;
    }
    let state = unsafe { &*(refcon as *const MacListenerState) };
    let notification_id = message_argument as isize;
    let connection = state
        .power_connection
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let connection = *connection;

    match message_type {
        K_IO_MESSAGE_CAN_SYSTEM_SLEEP => {
            native_log("native: kIOMessageCanSystemSleep (idle); allowing");
            allow_power_change(connection, notification_id);
        }
        K_IO_MESSAGE_SYSTEM_WILL_SLEEP => {
            native_log("native: kIOMessageSystemWillSleep");
            let _ = state.notify_sleep("kIOMessageSystemWillSleep");
            let completed = state.wait_for_sync("kIOMessageSystemWillSleep");
            native_log(&format!(
                "native: acknowledging sleep completed={completed}"
            ));
            allow_power_change(connection, notification_id);
        }
        K_IO_MESSAGE_SYSTEM_WILL_NOT_SLEEP => {
            native_log("native: kIOMessageSystemWillNotSleep");
        }
        K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON => {
            native_log("native: kIOMessageSystemHasPoweredOn");
            reset_sleep_handling();
        }
        K_IOPM_MESSAGE_CLAMSHELL_STATE_CHANGE => {
            let bits = message_argument as u32;
            let closed = bits & K_CLAMSHELL_STATE_BIT != 0;
            let will_sleep = bits & power::K_CLAMSHELL_SLEEP_BIT != 0;
            native_log(&format!(
                "native: clamshell state change bits=0x{bits:x} closed={closed} will_sleep={will_sleep}"
            ));
            if closed {
                let _ = state.notify_sleep("kIOPMMessageClamshellStateChange");
            }
        }
        other => {
            native_log(&format!("native: IOKit power message 0x{other:x}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::dispatch::CallbackDispatcher;

    #[test]
    fn macos_listener_starts_and_stops() {
        let mut platform = MacOsLifecycle::new().expect("platform");
        let (event_tx, _event_rx) = std::sync::mpsc::sync_channel(1);
        let (_ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let dispatcher = Arc::new(CallbackDispatcher::new());
        let sync_wait = Arc::new(SyncWait::new());

        platform
            .start(
                event_tx,
                dispatcher,
                ready_rx,
                sync_wait,
                Duration::from_secs(5),
            )
            .expect("start");
        std::thread::sleep(Duration::from_millis(400));
        platform.stop().expect("stop");
    }
}
