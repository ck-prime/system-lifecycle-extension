use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

pub struct SyncWait {
    completed: StdMutex<bool>,
    condvar: Condvar,
}

impl SyncWait {
    pub fn new() -> Self {
        Self {
            completed: StdMutex::new(false),
            condvar: Condvar::new(),
        }
    }

    pub fn signal(&self) -> bool {
        let mut guard = match self.completed.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if *guard {
            return false;
        }
        *guard = true;
        self.condvar.notify_all();
        true
    }

    pub fn wait(&self, timeout: Duration) -> bool {
        let guard = match self.completed.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if *guard {
            return true;
        }
        let (guard, wait_result) = self
            .condvar
            .wait_timeout(guard, timeout)
            .unwrap_or_else(|e| e.into_inner());
        *guard || !wait_result.timed_out()
    }

    pub fn reset(&self) {
        if let Ok(mut guard) = self.completed.lock() {
            *guard = false;
        }
    }
}
