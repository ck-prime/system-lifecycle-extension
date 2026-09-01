# System Lifecycle Extension — Architecture

## Purpose

This project is a **native Python extension** (PyO3 + maturin) that detects operating-system lifecycle transitions (shutdown, reboot, sleep, hibernate) and gives the embedding Python application a bounded window to finish synchronization before the transition proceeds.

It is **not** a standalone tracker application. Python owns business logic; Rust owns OS integration.

## High-Level Flow

```text
OS lifecycle event
       │
       ▼
Platform listener (background thread)
       │
       ▼
emit_lifecycle_event: worker enters WaitingForSync → dispatch Python callback
       │
       ▼
LifecycleController worker: acquire guard → wait synccomplete() (no GIL)
       │
       ▼
Release platform guard/inhibitor
       │
       ▼
OS transition continues
```

On **Windows shutdown**, the message loop also **blocks on `WM_ENDSESSION`** until sync completes, then posts quit so the process can exit cleanly.

On **macOS**, a dedicated CFRunLoop thread registers IOKit power notifications and an accessory `NSApplication`. The process is transformed to a UIElement app. **Sleep / lid close** delays `IOAllowPowerChange` until sync (same role as `WM_ENDSESSION`). **Shutdown** is detected via loginwindow `notify(3)` (`shutdownInitiated`) and held until `synccomplete()` or timeout.

## Components

| Layer | Responsibility |
|-------|----------------|
| `bridge/` | PyO3 bindings, `LifecycleManager`, `PythonDispatchThread` (callback queue) |
| `lifecycle/dispatch.rs` | `emit_lifecycle_event`, rendezvous with worker ready signal |
| `lifecycle/sync_wait.rs` | Condvar used by worker, Windows `WM_ENDSESSION`, and macOS `IOAllowPowerChange` / terminate hold |
| `lifecycle/` | State machine, event types, platform trait |
| `lifecycle/linux/` | systemd-logind over D-Bus, delay inhibitors |
| `lifecycle/macos/` | CFRunLoop, IOKit sleep/lid (`IOAllowPowerChange`), NSApplication terminate hold, IOPM assertions |
| `lifecycle/windows/` | Hidden top-level window, session/power messages, shutdown block reason |
| `native_log.rs` | Optional `lifecycle_native.log` for shutdown debugging |
| `errors.rs` | Typed errors (`thiserror`) |
| `logging.rs` | `tracing` initialization |

## State Machine

```text
Idle
  │ platform event (single active session)
  ▼
EventDetected
  ▼
WaitingForSync ──timeout──► TimedOut ──► Released ──► Idle
  │ synccomplete()
  ▼
SyncCompleted ──► Released ──► Idle

Stopped (terminal, from manager.stop())
```

Rules:

- Only **one** active synchronization session at a time (per process).
- Duplicate OS notifications while `WaitingForSync` are logged and ignored.
- Windows adds a process-wide **shutdown handling** flag to ignore duplicate `WM_QUERYENDSESSION` delivery.
- `synccomplete()` is **idempotent**; only the first call completes an active wait.
- Callback exceptions are logged; the session remains until `synccomplete()` or timeout.
- Timeout always releases the OS guard.

## Threading & GIL Safety

| Thread | Role |
|--------|------|
| Platform listener | OS APIs (D-Bus, Win32 message loop, NSWorkspace observer) |
| Controller worker | State machine, guard acquire/release, `sync_wait` (no GIL) |
| Python dispatch thread | Runs user callback from a `queue.Queue` |
| Python main / other | Application code; may call `synccomplete()` |

Critical invariant: the controller worker **never** holds the Python GIL while waiting for `synccomplete()`.

Callback dispatch:

```text
platform thread → emit_lifecycle_event → worker ready → enqueue to Python thread
Python thread   → user callback(event)
worker          → sync_wait.wait(timeout)   [no GIL]
Python          → synccomplete() → notify
worker          → release guard
```

## Platform Abstraction

```rust
trait PlatformLifecycle: Send {
    fn start(
        &mut self,
        events: SyncSender<PlatformEvent>,
        dispatcher: Arc<CallbackDispatcher>,
        worker_ready: Receiver<()>,
        sync_wait: Arc<SyncWait>,
        max_wait: Duration,
    ) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn acquire_guard(&self, event: LifecycleEventType) -> Result<PlatformGuard>;
}
```

`PlatformGuard` releases OS-specific delay mechanisms after sync or timeout.

## Windows shutdown sequence (validated)

1. `WM_QUERYENDSESSION`: `ShutdownBlockReasonCreate`, emit event, return **TRUE**.
2. Python callback runs on dispatch thread; worker waits on `sync_wait`.
3. `WM_ENDSESSION` (ending): message loop waits on same `sync_wait`.
4. `synccomplete()`: worker releases guard (`ShutdownBlockReasonDestroy`); ENDSESSION wait completes.
5. Host process should **exit** so shutdown proceeds (test bundle uses `os._exit(0)`).

## macOS shutdown and lid-close sequence

1. Dedicated listener thread runs `CFRunLoop` (NSWorkspace needs a run loop; IOKit sources are attached to it).
2. **Lid close**: `kIOPMMessageClamshellStateChange` with `kClamshellStateBit` → emit `sleep`, take `PreventSystemSleep`.
3. **Sleep** (lid, Apple menu, or idle completing): `kIOMessageSystemWillSleep` → emit if not already in-flight, **wait on `sync_wait`**, then `IOAllowPowerChange`.
4. **Shutdown / restart**: `applicationShouldTerminate` (and `NSWorkspaceWillPowerOffNotification`) → emit `shutdown` or `reboot`, wait on `sync_wait`, return `NSTerminateNow`.
5. After shutdown/reboot sync, the host process should **exit** (`os._exit(0)` in the test script).

Windows code paths are unchanged; macOS uses the same `emit_lifecycle_event` / `SyncWait` contract.

## Python API

```python
from system_lifecycle import LifecycleManager

manager = LifecycleManager(max_wait_seconds=90)
manager.start(on_system_event)
manager.synccomplete()  # returns True if a wait was active
manager.stop()
```

Events expose `event.type` (`shutdown`, `reboot`, `sleep`, `hibernate`) and `event.timestamp` (UTC epoch seconds).

## Testing Strategy

| Tier | Scope | Runs in CI |
|------|-------|------------|
| Unit | State machine, mock platform | Yes |
| Integration | Rust ↔ Python callback/synccomplete | Yes (mock platform) |
| Real OS | Manual shutdown/sleep | No |

Windows manual validation: `bundle/system-lifecycle-windows-test-bundle/` (see [testing.md](testing.md)).

## Build & Packaging

- **Development:** `maturin develop`
- **Production:** `maturin build --release` → wheel with native `.so`/`.dylib`/`.pyd`
- End users install wheels; **no Rust toolchain required at runtime**.

## Security

- No shell commands, no `sudo`, no modification of system shutdown configuration.
- Logs contain lifecycle metadata only.

## Platform Honesty

See [platform-support.md](platform-support.md) for per-OS capabilities, validated behavior, and limits.
