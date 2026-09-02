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

### Production embedding (Windows, macOS, and Linux)

1. **Run as your application process** — do not rely on a parent shell (CMD, IDE terminal) staying open. The process that calls `LifecycleManager.start()` is the one that delays shutdown.
2. In the lifecycle callback, call `synccomplete()` after durable work finishes.
3. On **shutdown/reboot**, **exit the process** after sync so the OS can finish the session (see the test bundles for a reference pattern).
4. On **Linux**, call `LifecycleManager.start()` early in the process lifetime so standby logind delay inhibitors are installed before shutdown or sleep is requested.

## Platform status

| Platform | Detection | Delay / block | Manual validation |
|----------|-----------|---------------|-----------------|
| **Linux** | logind `PrepareForShutdown` / `PrepareForSleep` | Standby **delay inhibitor** fds (`shutdown` + `sleep`) held from `start()` | **Shutdown + lid/sleep validated** (see test bundle) |
| **macOS** | IOKit clamshell/`WillSleep`, loginwindow notify | sleep/lid: `IOAllowPowerChange` + `PreventSystemSleep`; shutdown: UIElement app + loginwindow notify hold | **Shutdown + lid/sleep validated** (see test bundle) |
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
| **Linux x86_64** | systemd-logind session; cp310 test bundle |
| macOS x86_64 / arm64 | Sleep/lid + shutdown validated; test bundles for **cp314** and **cp310** |
| Windows x86_64 | Win10/Win11; cp314 wheel in test bundle |

## Python versions

The library supports **Python 3.10+** (`requires-python = ">=3.10"`). Use a matching test bundle or build a wheel for your interpreter.

`PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1` is only so PyO3 can **compile on 3.14**. It does not make one wheel work on 3.10–3.14.

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

## macOS manual test bundle

Prebuilt wheel + scripts for validating shutdown and lid close on a real machine (not CI):

```text
bundle/system-lifecycle-macos-test-bundle/          # CPython 3.14
  wheels/system_lifecycle-*-cp314-*-macosx_*.whl
  install.sh
  run_test_detached.sh
  test_lifecycle.py
  INSTALL.txt

bundle/system-lifecycle-macos-py310-test-bundle/   # CPython 3.10
  wheels/system_lifecycle-*-cp310-*-macosx_*.whl
  install.sh
  run_test_detached.sh
  test_lifecycle.py
  INSTALL.txt
```

Quick start (3.14):

```bash
cd bundle/system-lifecycle-macos-test-bundle
chmod +x install.sh run_test_detached.sh
./install.sh
./run_test_detached.sh
# Apple menu -> Shut Down, or close the lid
# After wake/reboot, check log.txt and lifecycle_native.log
```

Quick start (3.10):

```bash
cd bundle/system-lifecycle-macos-py310-test-bundle
chmod +x install.sh run_test_detached.sh
./install.sh
./run_test_detached.sh
# Apple menu -> Shut Down, or close the lid
# After wake/reboot, check log.txt and lifecycle_native.log
```

Successful shutdown test:

```text
event detected: type=shutdown
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

Native log should include `shutdownInitiated sync finished completed=true` (loginwindow notify) and, for lid close, `IOAllowPowerChange`.

## Linux manual test bundle (CPython 3.10)

Prebuilt wheel + scripts for validating shutdown and lid close / sleep on a real machine (not CI):

```text
bundle/system-lifecycle-linux-py310-test-bundle/
  wheels/system_lifecycle-*-cp310-*-manylinux_*.whl
  install.sh
  run_test_detached.sh
  test_lifecycle.py
  INSTALL.txt
```

Quick start:

```bash
cd bundle/system-lifecycle-linux-py310-test-bundle
chmod +x install.sh run_test_detached.sh
./install.sh
./run_test_detached.sh
# Desktop Shut Down, or close the lid
# After wake/reboot, check log.txt and lifecycle_native.log
```

Build from source (optional):

```bash
scripts/build_linux.sh   # builds cp310 wheel into dist/ and copies to bundle
```

Successful shutdown or lid/sleep test:

```text
event detected: type=shutdown   # or type=sleep for lid close
waiting for 6 seconds
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

Native log should include standby inhibitor install, active inhibitor on signal, and `bridge: sync wait finished completed=true`.

**Do not** run the test from inside Cursor for shutdown tests — use the detached launcher. Do not use `systemctl poweroff -i` (bypasses inhibitors).

## Other test scripts

After `maturin develop`:

```bash
python python/test_lifecycle.py
```

Interactive script with a 60-second sync window. Timeout, duplicate `synccomplete()`, and callback exceptions are covered by `cargo test --test lifecycle_integration`.

See [docs/testing.md](docs/testing.md) for the full manual OS matrix and bundle procedures.

## Documentation

- [Architecture](docs/architecture.md)
- [Platform support](docs/platform-support.md)
- [Testing](docs/testing.md)
- [Failure scenarios](docs/failure-scenarios.md)

## License

MIT — see [LICENSE](LICENSE).
