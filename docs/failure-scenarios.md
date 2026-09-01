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

1. Timeout fires (`max_wait_seconds`, default 15s).
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

## logind / D-Bus unavailable (Linux)

- Listener logs unavailability.
- Startup may fail platform listener initialization.
- Application should treat lifecycle protection as unavailable and rely on durable local storage.

## macOS shutdown delay unavailable

- Shutdown/reboot is still **detected** and surfaced to Python.
- True OS-level shutdown delay is not guaranteed; sleep uses IOPM assertions.

## Windows session without message pump

- Hidden message window requires a running listener thread.
- If window creation fails, errors are logged and lifecycle protection is unavailable.

## Extension internal errors

- Errors are converted to `RuntimeError` for Python API calls where appropriate.
- Production code avoids panics in lifecycle paths.
- Safest fallback: stop waiting after timeout and allow OS transition.

## Data safety principle

The extension is not the primary persistence layer. If sync fails, locally persisted application data should remain on disk; the extension only provides a final bounded safety window before power-off.
