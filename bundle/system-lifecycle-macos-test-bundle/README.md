# macOS Test Bundle

Manual validation package for **system_lifecycle** on macOS (Intel / Apple Silicon).

**Validated on Apple Silicon:** lid/sleep and Apple menu → Shut Down both reach `synccomplete()` (6s sync) before the OS continues.

The bundled wheel is **CPython 3.14** (`cp314`). For Python 3.10 use [`../system-lifecycle-macos-py310-test-bundle/`](../system-lifecycle-macos-py310-test-bundle/).

## Quick start

```bash
cd bundle/system-lifecycle-macos-test-bundle
chmod +x install.sh run_test_detached.sh
./install.sh
./run_test_detached.sh
```

Use **Apple menu → Shut Down**, or close the lid. After wake/reboot, open `log.txt` and `lifecycle_native.log`.

`install.sh` creates a local `.venv` (Homebrew Python is PEP 668 and cannot install this wheel system-wide).

See [INSTALL.txt](INSTALL.txt) for full instructions.

## Success criteria

**log.txt**

```text
event detected: type=shutdown
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

or for lid close:

```text
event detected: type=sleep
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

Single `event detected` block for shutdown (process exits). Sleep may continue heartbeats after wake.

**lifecycle_native.log**

Shutdown:

```text
native: TransformProcessType UIElement status=0
native: loginwindow notify(3) tokens registered
native: notify(3) fired name=com.apple.system.loginwindow.shutdownInitiated
native: shutdown detected source=com.apple.system.loginwindow.shutdownInitiated event=shutdown
native: com.apple.system.loginwindow.shutdownInitiated waiting for synccomplete
native: com.apple.system.loginwindow.shutdownInitiated sync finished completed=true
```

Lid / sleep:

```text
native: kIOMessageSystemWillSleep
native: kIOMessageSystemWillSleep waiting for synccomplete
native: kIOMessageSystemWillSleep sync finished completed=true
native: IOAllowPowerChange
```

## Do not

- Run `python3 test_lifecycle.py` inside Cursor for shutdown tests.
- Start the test twice without killing the previous process (`kill "$(cat .lifecycle_test.lock)"`).
- Use `sudo shutdown` (bypasses the normal app/loginwindow shutdown flow).
- Install the bundled `cp314` wheel into Python 3.10 (use the [py310 bundle](../system-lifecycle-macos-py310-test-bundle/) instead).

## Files

| File | Purpose |
|------|---------|
| `wheels/*.whl` | Prebuilt extension (cp314) |
| `install.sh` | Create local `.venv` and install the wheel |
| `run_test_detached.sh` | Launch test detached (recommended) |
| `test_lifecycle.py` | 6s sync test with logging |
| `log.txt` | Python output |
| `lifecycle_native.log` | Native bridge debug log |
| `.lifecycle_test.lock` | Single-instance lock (created at runtime) |
