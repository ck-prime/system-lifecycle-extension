use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::event::LifecycleEventType;
use crate::native_log::native_log;
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub type IoObject = u32;
pub type IoConnect = u32;
pub type IoService = u32;
pub type IoReturn = i32;
pub type IoNotificationPortRef = *mut c_void;

pub const K_IO_RETURN_SUCCESS: IoReturn = 0;
pub const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;

/// Values from IOKit/IOMessage.h (`iokit_common_msg`).
pub const K_IO_MESSAGE_CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
pub const K_IO_MESSAGE_SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;
pub const K_IO_MESSAGE_SYSTEM_WILL_NOT_SLEEP: u32 = 0xE000_0290;
pub const K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON: u32 = 0xE000_0300;

/// `kIOPMMessageClamshellStateChange` from IOPM.h (`iokit_family_msg`).
pub const K_IOPM_MESSAGE_CLAMSHELL_STATE_CHANGE: u32 = 0xE003_4100;
pub const K_CLAMSHELL_STATE_BIT: u32 = 1 << 0;
pub const K_CLAMSHELL_SLEEP_BIT: u32 = 1 << 1;

pub const K_IO_GENERAL_INTEREST: &[u8] = b"IOGeneralInterest\0";
pub const K_IOPM_ROOT_DOMAIN: &[u8] = b"IOPMrootDomain\0";

static SHUTDOWN_HANDLING: AtomicBool = AtomicBool::new(false);
static SLEEP_HANDLING: AtomicBool = AtomicBool::new(false);
static SLEEP_ASSERTIONS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    pub fn IOPMAssertionCreateWithName(
        assertion_type: *const c_void,
        assertion_level: u32,
        assertion_name: *const c_void,
        assertion_id: *mut u32,
    ) -> IoReturn;

    pub fn IOPMAssertionRelease(assertion_id: u32) -> IoReturn;

    pub fn IORegisterForSystemPower(
        refcon: *mut c_void,
        the_port_ref: *mut IoNotificationPortRef,
        callback: IOServiceInterestCallback,
        notifier: *mut IoObject,
    ) -> IoConnect;

    pub fn IODeregisterForSystemPower(notifier: *mut IoObject) -> IoReturn;

    pub fn IOAllowPowerChange(kernel_port: IoConnect, notification_id: isize) -> IoReturn;

    pub fn IONotificationPortGetRunLoopSource(
        notify: IoNotificationPortRef,
    ) -> core_foundation::runloop::CFRunLoopSourceRef;

    pub fn IONotificationPortDestroy(notify: IoNotificationPortRef);

    pub fn IOServiceGetMatchingService(main_port: u32, matching: *const c_void) -> IoService;

    pub fn IOServiceMatching(name: *const i8) -> *mut c_void;

    pub fn IOServiceAddInterestNotification(
        notify_port: IoNotificationPortRef,
        service: IoService,
        interest_type: *const i8,
        callback: IOServiceInterestCallback,
        refcon: *mut c_void,
        notification: *mut IoObject,
    ) -> IoReturn;

    pub fn IOServiceClose(connect: IoConnect) -> IoReturn;

    pub fn IOObjectRelease(object: IoObject) -> IoReturn;
}

/// `0` is `kIOMainPortDefault` / `MACH_PORT_NULL` (use the default master port).
pub const K_IO_MAIN_PORT_DEFAULT: u32 = 0;

pub type IOServiceInterestCallback = unsafe extern "C" fn(
    refcon: *mut c_void,
    service: IoService,
    message_type: u32,
    message_argument: *mut c_void,
);

pub fn reset_shutdown_handling() {
    SHUTDOWN_HANDLING.store(false, Ordering::SeqCst);
}

pub fn begin_shutdown_handling() -> bool {
    !SHUTDOWN_HANDLING.swap(true, Ordering::SeqCst)
}

pub fn reset_sleep_handling() {
    SLEEP_HANDLING.store(false, Ordering::SeqCst);
}

pub fn begin_sleep_handling() -> bool {
    !SLEEP_HANDLING.swap(true, Ordering::SeqCst)
}

fn create_assertion(assertion_type: &str) -> LifecycleResult<u32> {
    let type_cf = CFString::new(assertion_type);
    let name = CFString::new("system-lifecycle-extension synchronization");
    let mut assertion_id = 0u32;
    let result = unsafe {
        IOPMAssertionCreateWithName(
            type_cf.as_concrete_TypeRef() as *const c_void,
            K_IOPM_ASSERTION_LEVEL_ON,
            name.as_concrete_TypeRef() as *const c_void,
            &mut assertion_id,
        )
    };
    if result != K_IO_RETURN_SUCCESS {
        return Err(LifecycleError::Platform(format!(
            "IOPMAssertionCreateWithName({assertion_type}) failed with code {result}"
        )));
    }
    native_log(&format!(
        "native: IOPM assertion created type={assertion_type} id={assertion_id}"
    ));
    Ok(assertion_id)
}

/// Hold system sleep (including lid-close sleep) until `release_sleep_power`.
///
/// `PreventUserIdleSystemSleep` only blocks idle sleep. Lid close requires
/// `PreventSystemSleep` plus delaying `IOAllowPowerChange` on WillSleep.
pub fn hold_sleep_power() -> LifecycleResult<()> {
    let mut held = SLEEP_ASSERTIONS
        .lock()
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;
    if !held.is_empty() {
        return Ok(());
    }

    let system = create_assertion("PreventSystemSleep")?;
    held.push(system);

    match create_assertion("PreventUserIdleSystemSleep") {
        Ok(idle) => held.push(idle),
        Err(err) => {
            native_log(&format!(
                "native: PreventUserIdleSystemSleep failed (continuing with PreventSystemSleep): {err}"
            ));
        }
    }

    Ok(())
}

pub fn release_sleep_power() -> LifecycleResult<()> {
    let mut held = SLEEP_ASSERTIONS
        .lock()
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;
    let mut first_error = None;
    for assertion_id in held.drain(..) {
        let result = unsafe { IOPMAssertionRelease(assertion_id) };
        if result != K_IO_RETURN_SUCCESS {
            let err = format!("IOPMAssertionRelease({assertion_id}) failed with code {result}");
            native_log(&format!("native: {err}"));
            if first_error.is_none() {
                first_error = Some(err);
            }
        } else {
            native_log(&format!(
                "native: IOPM assertion released id={assertion_id}"
            ));
        }
    }
    reset_sleep_handling();
    match first_error {
        Some(err) => Err(LifecycleError::Platform(err)),
        None => Ok(()),
    }
}

pub fn allow_power_change(kernel_port: IoConnect, notification_id: isize) {
    if kernel_port == 0 {
        native_log("native: IOAllowPowerChange skipped, port unavailable");
        return;
    }
    let result = unsafe { IOAllowPowerChange(kernel_port, notification_id) };
    native_log(&format!(
        "native: IOAllowPowerChange id={notification_id} result={result}"
    ));
}

pub struct MacOsGuard {
    event: LifecycleEventType,
    released: bool,
}

impl MacOsGuard {
    /// Delay mechanisms are taken in the listener callback before the event is
    /// queued; this guard only releases them after sync (mirrors Windows).
    pub fn new(event: LifecycleEventType) -> LifecycleResult<Self> {
        Ok(Self {
            event,
            released: false,
        })
    }
}

impl crate::lifecycle::platform::PlatformGuard for MacOsGuard {
    fn release(&mut self) -> LifecycleResult<()> {
        if self.released {
            return Ok(());
        }
        match self.event {
            LifecycleEventType::Sleep | LifecycleEventType::Hibernate => {
                release_sleep_power()?;
            }
            LifecycleEventType::Shutdown | LifecycleEventType::Reboot => {
                reset_shutdown_handling();
            }
        }
        self.released = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iokit_message_constants_match_sdk() {
        assert_eq!(K_IO_MESSAGE_CAN_SYSTEM_SLEEP, 0xE000_0270);
        assert_eq!(K_IO_MESSAGE_SYSTEM_WILL_SLEEP, 0xE000_0280);
        assert_eq!(K_IO_MESSAGE_SYSTEM_WILL_NOT_SLEEP, 0xE000_0290);
        assert_eq!(K_IOPM_MESSAGE_CLAMSHELL_STATE_CHANGE, 0xE003_4100);
        assert_eq!(K_CLAMSHELL_STATE_BIT, 1);
        assert_eq!(K_CLAMSHELL_SLEEP_BIT, 2);
        assert_eq!(K_IOPM_ASSERTION_LEVEL_ON, 255);
    }

    #[test]
    fn sleep_assertion_creates_and_releases() {
        hold_sleep_power().expect("PreventSystemSleep assertion should succeed");
        release_sleep_power().expect("assertion release should succeed");
    }

    #[test]
    fn begin_sleep_and_shutdown_flags_are_single_shot() {
        reset_sleep_handling();
        reset_shutdown_handling();
        assert!(begin_sleep_handling());
        assert!(!begin_sleep_handling());
        reset_sleep_handling();
        assert!(begin_sleep_handling());
        reset_sleep_handling();

        assert!(begin_shutdown_handling());
        assert!(!begin_shutdown_handling());
        reset_shutdown_handling();
        assert!(begin_shutdown_handling());
        reset_shutdown_handling();
    }
}
