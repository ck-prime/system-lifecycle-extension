use crate::lifecycle::event::LifecycleEvent;
use crate::native_log::native_log;
use parking_lot::Mutex;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use tracing::{error, warn};

/// Routes lifecycle callbacks through a dedicated Python thread.
///
/// Calling Python from a Rust worker during Windows shutdown is unreliable; a
/// real Python thread keeps the interpreter in a valid state for user code.
pub struct PythonDispatchThread {
    event_queue: Mutex<Option<Py<PyAny>>>,
    dispatch_thread: Mutex<Option<Py<PyAny>>>,
}

impl PythonDispatchThread {
    pub fn new() -> Self {
        Self {
            event_queue: Mutex::new(None),
            dispatch_thread: Mutex::new(None),
        }
    }

    pub fn start(&self, callback: Py<PyAny>) -> PyResult<()> {
        Python::with_gil(|py| {
            py.run(
                c"def _sl_start_dispatcher(callback):\n    import queue\n    import threading\n    event_queue = queue.Queue()\n    def worker():\n        while True:\n            item = event_queue.get()\n            if item is None:\n                break\n            callback(item)\n    thread = threading.Thread(\n        target=worker,\n        name='system-lifecycle-callback',\n        daemon=False,\n    )\n    thread.start()\n    return event_queue, thread\n",
                None,
                None,
            )?;
            let create = py.eval(c"_sl_start_dispatcher", None, None)?;
            let result = create.call1((callback,))?;
            let (queue, thread) = result.extract::<(Py<PyAny>, Py<PyAny>)>()?;

            *self.event_queue.lock() = Some(queue);
            *self.dispatch_thread.lock() = Some(thread);
            native_log("bridge: python dispatch thread started");
            Ok(())
        })
    }

    pub fn enqueue(&self, event: &LifecycleEvent) {
        let queue = self.event_queue.lock();
        let Some(queue) = queue.as_ref() else {
            native_log("bridge: python dispatch queue unavailable");
            warn!("python dispatch queue unavailable");
            return;
        };

        native_log(&format!(
            "bridge: enqueueing python callback event={}",
            event.event_type.as_str()
        ));

        if let Err(err) = Python::with_gil(|py| {
            let py_event = crate::bridge::event_to_py(py, event)?;
            queue.call_method1(py, "put", (py_event,))
        }) {
            native_log(&format!("bridge: failed to enqueue python callback: {err}"));
            error!(error = %err, "failed to enqueue python lifecycle callback");
        } else {
            native_log(&format!(
                "bridge: python callback enqueued event={}",
                event.event_type.as_str()
            ));
        }
    }

    pub fn stop(&self) {
        let queue = self.event_queue.lock().take();
        let thread = self.dispatch_thread.lock().take();
        let Some(queue) = queue else {
            return;
        };

        let _ = Python::with_gil(|py| {
            queue.call_method1(py, "put", (py.None(),))?;
            if let Some(thread) = thread {
                let _ = thread.call_method1(py, "join", PyTuple::new(py, [5.0])?)?;
            }
            Ok::<(), PyErr>(())
        });

        native_log("bridge: python dispatch thread stopped");
    }
}

impl Default for PythonDispatchThread {
    fn default() -> Self {
        Self::new()
    }
}
