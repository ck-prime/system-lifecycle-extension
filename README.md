# System Lifecycle Extension

Cross-platform **Rust/Python native extension** that detects operating-system lifecycle transitions (shutdown, reboot, sleep, hibernate) and gives your Python application a bounded window to finish synchronization before the OS continues.

This project is **not** a standalone tracker. It is a library extension consumed by a Python application.

## Architecture

```text
OS event -> platform listener -> state machine -> Python callback (dedicated thread)
                                      |
                                      v
                               wait synccomplete()
                                      |
                                      v
                               release OS guard -> OS continues
```

See [docs/architecture.md](docs/architecture.md) for the full design.

## Python API

```python
from system_lifecycle import LifecycleManager

manager = LifecycleManager(max_wait_seconds=90)

def on_system_event(event):
    print(event.type, event.timestamp)
    # perform durable sync work here
    manager.synccomplete()

manager.start(on_system_event)
# application continues running
manager.stop()
```

### `synccomplete()` semantics

- Returns `True` when an active lifecycle wait was completed.
- Returns `False` when no synchronization was pending (safe no-op; idempotent).
- The OS guard is **not** released when the callback returns; only `synccomplete()` or timeout releases it.

### Production embedding (especially Windows)

1. **Run as your application process** — do not rely on a parent shell (CMD, IDE terminal) staying open. The process that calls `LifecycleManager.start()` is the one that blocks shutdown.
2. In the lifecycle callback, call `synccomplete()` after durable work finishes.
3. On **shutdown/reboot**, **exit the process** after sync so Windows can finish the session (see the test bundle for a reference pattern).

## Platform status

| Platform | Detection | Delay / block | Manual validation |
|----------|-----------|---------------|-----------------|
| Linux | logind D-Bus | delay inhibitor | Documented; test in VM |
| macOS | NSWorkspace | sleep: IOPM; shutdown: notify only | Documented; test on hardware |
| **Windows 10/11** | `WM_QUERYENDSESSION`, power broadcasts | `ShutdownBlockReasonCreate`, `SetThreadExecutionState` | **Shutdown + sleep validated** (see test bundle) |

Details: [docs/platform-support.md](docs/platform-support.md)

## Installation (production)

Install a prebuilt wheel (no Rust toolchain required):

```bash
pip install system_lifecycle-*.whl
```

Build targets:

| Target | Notes |
|--------|-------|
| Linux x86_64 / aarch64 | systemd-logind session |
| macOS x86_64 / arm64 | Sleep delay via IOPM; shutdown notification only |
| Windows x86_64 | Win10/Win11; cp314 wheel in test bundle |

## Development build

Requirements: Rust stable, Python 3.10+, maturin

```bash
pip install maturin
export PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1  # Python 3.14+
maturin develop --release
cargo test --all-features
```

Wheels: `maturin build --release --out dist`

Platform scripts: `scripts/build_linux.sh`, `scripts/build_macos.sh`, `scripts/build_windows.ps1`

## Windows manual test bundle

Prebuilt wheel + scripts for validating shutdown on a real machine (not CI):

```text
bundle/system-lifecycle-windows-test-bundle/
  wheels/system_lifecycle-*-cp314-*-win_amd64.whl
  run_test_detached.ps1   # recommended
  run_test_detached.cmd
  test_lifecycle.py
  INSTALL.txt
```

Quick start:

```powershell
cd /d path\to\bundle\system-lifecycle-windows-test-bundle
py -3.14 -m pip install --force-reinstall wheels\system_lifecycle-0.1.0-cp314-cp314-win_amd64.whl
powershell -ExecutionPolicy Bypass -File run_test_detached.ps1
# Start menu -> Shut down
# After reboot, check log.txt and lifecycle_native.log
```

Successful shutdown test:

```text
event detected: type=shutdown
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

Native log should include `WM_ENDSESSION sync finished completed=true`.

**Do not** run the test from CMD/Cursor for shutdown tests — that adds extra processes to the shutdown UI. Use the detached launcher.

## Other test scripts

After `maturin develop`:

```bash
python python/test_lifecycle.py
```

Interactive script with a 60-second sync window. Timeout, duplicate `synccomplete()`, and callback exceptions are covered by `cargo test --test lifecycle_integration`.

See [docs/testing.md](docs/testing.md) for the full manual OS matrix and Windows bundle procedure.

## Documentation

- [Architecture](docs/architecture.md)
- [Platform support](docs/platform-support.md)
- [Testing](docs/testing.md)
- [Failure scenarios](docs/failure-scenarios.md)

## License

MIT — see [LICENSE](LICENSE).
