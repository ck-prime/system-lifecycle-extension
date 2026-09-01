# Failure Scenarios

## Python callback raises

Behavior:

1. Rust logs the exception.
2. Lifecycle state remains in `waiting_for_sync`.
3. Extension waits for explicit `synccomplete()` or `max_wait_seconds` timeout.
4. OS guard is released on sync or timeout.

The Python process must not crash.

## Python never calls `synccomplete()`

Behavior:

1. Timeout fires (`max_wait_seconds`).
2. Rust logs timeout.
3. OS guard/inhibitor is released.
4. OS transition continues.

The extension never blocks shutdown/sleep indefinitely.

## Duplicate `synccomplete()`

- First call during an active wait completes the session.
- Subsequent calls return `False` and do not double-release resources.

## Duplicate OS lifecycle events during active wait

- First event owns the synchronization cycle.
- Additional events are logged and ignored until the current cycle completes.
- On Windows, a process-wide shutdown flag ignores duplicate `WM_QUERYENDSESSION` in the same session.

## Multiple test / app instances (Windows)

- Each process registers its own shutdown block reason.
- Running the test twice (e.g. detached + CMD) causes duplicate handlers and a cluttered shutdown UI.
- Use a single-instance lock in the test script; production apps should enforce one listener per process.

## logind / D-Bus unavailable (Linux)

- Listener logs unavailability.
- Startup may fail platform listener initialization.
- Application should treat lifecycle protection as unavailable and rely on durable local storage.

## macOS shutdown / lid delay limits

- Shutdown is detected via loginwindow `notify(3)` (`com.apple.system.loginwindow.shutdownInitiated`) after the process is transformed to a UIElement app. SIGTERM alone is too late (loginwindow SIGKILLs before a 6s sync can finish).
- Delay is a hold until `synccomplete()` or timeout. The OS can still force-quit after its own logout limit. This is weaker than Windows `ShutdownBlockReasonCreate`.
- Lid close is detected via `kIOPMMessageClamshellStateChange` and delayed by holding `IOAllowPowerChange` on `kIOMessageSystemWillSleep` (about 30 seconds of OS slack if the app never acknowledges).
- `PreventUserIdleSystemSleep` alone does **not** delay lid-close sleep; the extension uses `PreventSystemSleep` plus the IOKit acknowledge hold.

## Windows session without message pump

- Hidden top-level window requires a running listener thread.
- If window creation fails, errors are logged and lifecycle protection is unavailable.

## Windows: sync completes but shutdown stalls

If `synccomplete()` succeeds but the machine does not shut down:

- The **host process may still be running** (e.g. main thread in an infinite loop).
- Production apps should exit after shutdown/reboot sync so Windows can finish the session.
- The test bundle calls `os._exit(0)` after sync as a reference pattern.

## Extension internal errors

- Errors are converted to `RuntimeError` for Python API calls where appropriate.
- Safest fallback: stop waiting after timeout and allow OS transition.

## Data safety principle

The extension is not the primary persistence layer. If sync fails, locally persisted application data should remain on disk; the extension only provides a final bounded safety window before power-off.
