use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::event::LifecycleEventType;
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use std::ffi::c_void;

type IOPMAssertionID = u32;
type IOReturn = i32;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: *const c_void,
        assertion_level: u32,
        assertion_name: *const c_void,
        assertion_id: *mut IOPMAssertionID,
    ) -> IOReturn;

    fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
}

extern "C" {
    static kIOPMAssertionTypePreventUserIdleSystemSleep: *const c_void;
}

pub struct MacOsGuard {
    assertion_id: IOPMAssertionID,
    released: bool,
}

impl MacOsGuard {
    pub fn new(event: LifecycleEventType) -> LifecycleResult<Self> {
        match event {
            LifecycleEventType::Sleep | LifecycleEventType::Hibernate => {
                let name = CFString::new("system-lifecycle-extension synchronization");
                let mut assertion_id = 0;
                let result = unsafe {
                    IOPMAssertionCreateWithName(
                        kIOPMAssertionTypePreventUserIdleSystemSleep,
                        0,
                        name.as_concrete_TypeRef() as *const c_void,
                        &mut assertion_id,
                    )
                };
                if result != 0 {
                    return Err(LifecycleError::Platform(format!(
                        "IOPMAssertionCreateWithName failed with code {result}"
                    )));
                }
                Ok(Self {
                    assertion_id,
                    released: false,
                })
            }
            LifecycleEventType::Shutdown | LifecycleEventType::Reboot => {
                // macOS does not provide a Linux-style delay inhibitor for shutdown.
                // Notification is supported; bounded delay relies on application response time.
                Ok(Self {
                    assertion_id: 0,
                    released: false,
                })
            }
        }
    }
}

impl crate::lifecycle::platform::PlatformGuard for MacOsGuard {
    fn release(&mut self) -> LifecycleResult<()> {
        if self.released {
            return Ok(());
        }
        if self.assertion_id != 0 {
            let result = unsafe { IOPMAssertionRelease(self.assertion_id) };
            if result != 0 {
                return Err(LifecycleError::Platform(format!(
                    "IOPMAssertionRelease failed with code {result}"
                )));
            }
            self.assertion_id = 0;
        }
        self.released = true;
        Ok(())
    }
}
