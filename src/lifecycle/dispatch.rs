use crate::bridge::python_dispatch::PythonDispatchThread;
use crate::lifecycle::event::{LifecycleEvent, LifecycleEventType, PlatformEvent};
use crate::native_log::native_log;
use pyo3::prelude::*;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::time::Duration;

pub struct CallbackDispatcher {
    python_dispatch: PythonDispatchThread,
}

impl CallbackDispatcher {
    pub fn new() -> Self {
        Self {
            python_dispatch: PythonDispatchThread::new(),
        }
    }

    pub fn start(&self, callback: Py<PyAny>) -> PyResult<()> {
        self.python_dispatch.start(callback)
    }

    pub fn clear_callback(&self) {
        self.python_dispatch.stop();
    }

    pub fn dispatch(&self, event: &LifecycleEvent) {
        self.python_dispatch.enqueue(event);
    }
}

impl Default for CallbackDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Hand event to the worker, wait until it enters `WaitingForSync`, then notify Python.
pub fn emit_lifecycle_event(
    events: &SyncSender<PlatformEvent>,
    worker_ready: &Receiver<()>,
    dispatcher: &CallbackDispatcher,
    event_type: LifecycleEventType,
    source: &str,
) {
    let platform_event = PlatformEvent::new(event_type);
    native_log(&format!(
        "bridge: emitting lifecycle event source={source} event={}",
        event_type.as_str()
    ));

    if let Err(err) = events.send(platform_event.clone()) {
        native_log(&format!("bridge: failed to deliver event to worker: {err}"));
        return;
    }

    match worker_ready.recv_timeout(Duration::from_secs(5)) {
        Ok(()) => native_log("bridge: worker ready for python dispatch"),
        Err(RecvTimeoutError::Timeout) => {
            native_log("bridge: timed out waiting for worker ready signal");
            return;
        }
        Err(RecvTimeoutError::Disconnected) => {
            native_log("bridge: worker ready channel disconnected");
            return;
        }
    }

    dispatcher.dispatch(&platform_event.event);
}
