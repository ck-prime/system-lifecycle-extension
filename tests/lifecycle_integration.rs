use pyo3::prelude::*;
use pyo3::types::PyList;
use std::time::Duration;
use system_lifecycle::lifecycle::controller::LifecycleController;
use system_lifecycle::lifecycle::event::LifecycleEventType;
use system_lifecycle::lifecycle::mock::MockLifecycle;

fn install_callback(py: Python<'_>) -> PyResult<Py<PyAny>> {
    py.run(
        c"state = [False]\ndef _cb(event):\n    state[0] = True\n",
        None,
        None,
    )?;
    py.eval(c"_cb", None, None).map(|obj| obj.into())
}

fn callback_state(py: Python<'_>) -> PyResult<bool> {
    let state: Bound<'_, PyList> = py.eval(c"state", None, None)?.extract()?;
    state.get_item(0)?.extract()
}

#[test]
fn mock_lifecycle_callback_and_synccomplete() {
    pyo3::prepare_freethreaded_python();

    let mock = MockLifecycle::new();
    let inject = mock.inject_handle();
    let controller = LifecycleController::new(Duration::from_secs(2), Box::new(mock));

    Python::with_gil(|py| {
        let callback = install_callback(py).unwrap();
        controller.start(callback).unwrap();
    });

    inject.inject_event(LifecycleEventType::Shutdown).unwrap();

    let mut called = false;
    for _ in 0..50 {
        Python::with_gil(|py| {
            called = callback_state(py).unwrap();
        });
        if called {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(called);

    assert!(controller.sync_complete());
    assert!(!controller.sync_complete());

    controller.stop().unwrap();
}

#[test]
fn mock_lifecycle_timeout_releases_wait() {
    pyo3::prepare_freethreaded_python();

    let mock = MockLifecycle::new();
    let inject = mock.inject_handle();
    let controller = LifecycleController::new(Duration::from_millis(200), Box::new(mock));

    Python::with_gil(|py| {
        let callback = install_callback(py).unwrap();
        controller.start(callback).unwrap();
    });

    inject.inject_event(LifecycleEventType::Sleep).unwrap();
    std::thread::sleep(Duration::from_millis(500));

    assert!(!controller.sync_complete());
    controller.stop().unwrap();
}

#[test]
fn duplicate_synccomplete_is_safe() {
    let controller =
        LifecycleController::new(Duration::from_secs(1), Box::new(MockLifecycle::new()));
    assert!(!controller.sync_complete());
    assert!(!controller.sync_complete());
}

#[test]
fn callback_exception_does_not_crash_extension() {
    pyo3::prepare_freethreaded_python();

    let mock = MockLifecycle::new();
    let inject = mock.inject_handle();
    let controller = LifecycleController::new(Duration::from_millis(200), Box::new(mock));

    Python::with_gil(|py| {
        py.run(
            std::ffi::CString::new("def _cb(event):\n    raise RuntimeError('boom')")
                .unwrap()
                .as_c_str(),
            None,
            None,
        )
        .unwrap();
        let callback = py.eval(c"_cb", None, None).unwrap().into();
        controller.start(callback).unwrap();
    });

    inject.inject_event(LifecycleEventType::Shutdown).unwrap();
    std::thread::sleep(Duration::from_millis(400));

    assert!(!controller.sync_complete());
    controller.stop().unwrap();
}
