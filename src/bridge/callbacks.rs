use crate::errors::LifecycleError;
use crate::lifecycle::event::LifecycleEvent;
use crate::lifecycle::event::LifecycleEventType;
use pyo3::prelude::*;

#[pyclass(name = "LifecycleEventType", eq, eq_int)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyLifecycleEventType {
    Shutdown,
    Reboot,
    Sleep,
    Hibernate,
}

impl From<LifecycleEventType> for PyLifecycleEventType {
    fn from(value: LifecycleEventType) -> Self {
        match value {
            LifecycleEventType::Shutdown => Self::Shutdown,
            LifecycleEventType::Reboot => Self::Reboot,
            LifecycleEventType::Sleep => Self::Sleep,
            LifecycleEventType::Hibernate => Self::Hibernate,
        }
    }
}

#[pymethods]
impl PyLifecycleEventType {
    #[getter]
    fn value(&self) -> &'static str {
        match self {
            Self::Shutdown => "shutdown",
            Self::Reboot => "reboot",
            Self::Sleep => "sleep",
            Self::Hibernate => "hibernate",
        }
    }

    fn __repr__(&self) -> String {
        format!("LifecycleEventType('{}')", self.value())
    }

    fn __str__(&self) -> &'static str {
        self.value()
    }
}

#[pyclass(name = "LifecycleEvent")]
#[derive(Clone)]
pub struct PyLifecycleEvent {
    #[pyo3(get)]
    pub r#type: PyLifecycleEventType,
    #[pyo3(get)]
    pub timestamp: u64,
}

#[pymethods]
impl PyLifecycleEvent {
    fn __repr__(&self) -> String {
        format!(
            "LifecycleEvent(type='{}', timestamp={})",
            self.r#type.value(),
            self.timestamp
        )
    }
}

pub fn event_to_py(py: Python<'_>, event: &LifecycleEvent) -> PyResult<Py<PyLifecycleEvent>> {
    Py::new(
        py,
        PyLifecycleEvent {
            r#type: event.event_type.into(),
            timestamp: event.timestamp,
        },
    )
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyLifecycleEventType>()?;
    m.add_class::<PyLifecycleEvent>()?;
    Ok(())
}

pub fn lifecycle_error_to_py(err: LifecycleError) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(err.to_string())
}
