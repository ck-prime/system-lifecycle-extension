# System Lifecycle Extension

Cross-platform **Rust/Python native extension** that detects operating-system lifecycle transitions (shutdown, reboot, sleep, hibernate) and gives your Python application a bounded window to finish synchronization before the OS continues.

This project is **not** a standalone tracker. It is a library extension consumed by a Python application.

## Architecture

```text
OS event -> platform listener -> state machine -> Python callback
                                      |
                                      v
                               wait synccomplete()
                                      |
                                      v
                               release OS guard
```

See [docs/architecture.md](docs/architecture.md) for the full design.

## Python API

```python
from system_lifecycle import LifecycleManager

manager = LifecycleManager(max_wait_seconds=15)

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

## Installation (production)

Install a prebuilt wheel (no Rust toolchain required):

```bash
pip install system_lifecycle-*.whl
```

Supported build targets:

| Target | Notes |
|--------|-------|
| Linux x86_64 | Ubuntu 22.04/24.04/26.04 + systemd-logind |
| Linux aarch64 | Same requirements on ARM64 distros |
| macOS x86_64 / arm64 | Sleep delay via IOPM; shutdown notification only |
| Windows x86_64 | Win10/Win11 session/power messages |

## Development build

Requirements:

- Rust stable
- Python 3.10+
- maturin

```bash
pip install maturin
export PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1  # if using Python 3.14+
maturin develop --release
```

## Production build

```bash
maturin build --release
```

Wheels are written to `target/wheels/`.

Platform helper scripts:

- `scripts/build_linux.sh`
- `scripts/build_macos.sh`
- `scripts/build_windows.ps1`

## Linux setup

Uses **systemd-logind** over system D-Bus (`org.freedesktop.login1`):

- Signals: `PrepareForShutdown`, `PrepareForSleep`
- Delay mechanism: logind **delay inhibitor** fd

Requires a desktop/session environment with logind (normal Ubuntu desktop/server with systemd). No root privileges required for standard session inhibition.

## macOS setup

Uses **NSWorkspace** notifications and **IOPM assertions** for sleep delay.

Shutdown/reboot notification is supported; macOS does not provide a Linux-equivalent shutdown delay inhibitor. See platform limitations.

## Windows setup

Uses a hidden message-only window and native session/power messages:

- `WM_QUERYENDSESSION` + `ShutdownBlockReasonCreate` for shutdown/reboot delay
- `WM_POWERBROADCAST` + `SetThreadExecutionState` for sleep delay

No Windows service is created by the extension itself.

## Test Python script

After `maturin develop`:

```bash
python python/test_lifecycle.py --sync-delay 5
python python/test_lifecycle.py --sync-delay 0
python python/test_lifecycle.py --sync-delay 20
python python/test_lifecycle.py --never-complete
python python/test_lifecycle.py --raise-error
```

Then manually initiate shutdown/sleep to exercise real OS events.

## Manual OS lifecycle testing

Never automated in CI.

1. Run `python/test_lifecycle.py` in a VM.
2. Manually trigger shutdown, reboot, or sleep.
3. Verify callback, `synccomplete()`, timeout behavior, and that the OS transition continues.

See [docs/testing.md](docs/testing.md) and the VM matrix there.

## Failure scenarios

See [docs/failure-scenarios.md](docs/failure-scenarios.md).

## Platform limitations

See [docs/platform-support.md](docs/platform-support.md) for an honest per-OS capability matrix.

## License

MIT — see [LICENSE](LICENSE).
