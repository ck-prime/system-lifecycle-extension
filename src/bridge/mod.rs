mod callbacks;
pub(crate) mod python_dispatch;

pub use callbacks::{event_to_py, lifecycle_error_to_py, register_module};

use crate::errors::LifecycleError;
use crate::lifecycle::controller::LifecycleController;
use crate::lifecycle::platform::create_platform;
use parking_lot::Mutex;
use pyo3::prelude::*;
use std::sync::Arc;
use std::time::Duration;

#[pyclass]
pub struct LifecycleManager {
    controller: Arc<LifecycleController>,
    started: Mutex<bool>,
}

#[pymethods]
impl LifecycleManager {
    #[new]
    #[pyo3(signature = (max_wait_seconds=15))]
    fn new(max_wait_seconds: u64) -> PyResult<Self> {
        crate::logging::init_logging();
        let platform = create_platform().map_err(lifecycle_error_to_py)?;
        Ok(Self {
            controller: Arc::new(LifecycleController::new(
                Duration::from_secs(max_wait_seconds),
                platform,
            )),
            started: Mutex::new(false),
        })
    }

    fn start(&self, callback: Py<PyAny>) -> PyResult<()> {
        let mut started = self.started.lock();
        if *started {
            return Err(lifecycle_error_to_py(LifecycleError::AlreadyRunning));
        }
        self.controller
            .start(callback)
            .map_err(lifecycle_error_to_py)?;
        *started = true;
        Ok(())
    }

    fn synccomplete(&self) -> PyResult<bool> {
        Ok(self.controller.sync_complete())
    }

    fn stop(&self) -> PyResult<()> {
        let mut started = self.started.lock();
        if !*started {
            return Err(lifecycle_error_to_py(LifecycleError::NotRunning));
        }
        self.controller.stop().map_err(lifecycle_error_to_py)?;
        *started = false;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.controller.is_running()
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)?;
    m.add_class::<LifecycleManager>()?;
    Ok(())
}
