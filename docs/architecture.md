# System Lifecycle Extension — Architecture

## Purpose

This project is a **native Python extension** (PyO3 + maturin) that detects operating-system lifecycle transitions (shutdown, reboot, sleep, hibernate) and gives the embedding Python application a bounded window to finish synchronization before the transition proceeds.

It is **not** a standalone tracker application. Python owns business logic; Rust owns OS integration.

## High-Level Flow

```
OS lifecycle event
       │
       ▼
Platform listener (background thread)
       │
       ▼
LifecycleController state machine
       │
       ├── acquire platform guard/inhibitor
       ├── dispatch Python callback (GIL held only for callback)
       └── wait for synccomplete() OR timeout (NO GIL)
       │
       ▼
Release platform guard/inhibitor
       │
       ▼
OS transition continues
```

## Components

| Layer | Responsibility |
|-------|----------------|
| `bridge/` | PyO3 bindings, `LifecycleManager`, Python callback dispatch |
| `lifecycle/` | State machine, event types, platform trait |
| `lifecycle/linux/` | systemd-logind over D-Bus, delay inhibitors |
| `lifecycle/macos/` | IOKit / NSWorkspace power notifications |
| `lifecycle/windows/` | Hidden message window, session/power messages |
| `errors.rs` | Typed errors (`thiserror`) |
| `logging.rs` | `tracing` initialization |

## State Machine

```
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

- Only **one** active synchronization session at a time.
- Duplicate OS notifications while `WaitingForSync` are logged and ignored.
- `synccomplete()` is **idempotent**; only the first call completes an active wait.
- Callback exceptions are logged; the session remains until `synccomplete()` or timeout.
- Timeout always releases the OS guard — shutdown/sleep is never blocked indefinitely.

## Threading & GIL Safety

Three roles:

1. **Platform listener thread** — blocks on OS APIs (D-Bus, Win32 message loop, CFRunLoop).
2. **Controller worker thread** — owns the state machine, acquires/releases guards, waits on a `Condvar` for sync completion.
3. **Python threads** — run the user callback and may call `synccomplete()`.

Critical invariant: the controller worker **never** holds the Python GIL while waiting for `synccomplete()`.

Callback dispatch:

```
worker ──► Python::attach ──► call callback ──► release GIL
worker ──► condvar.wait(timeout)
Python ──► synccomplete() ──► condvar.notify
worker ──► release guard
```

## Platform Abstraction

```rust
trait PlatformLifecycle: Send {
    fn start(&mut self, events: Sender<PlatformEvent>) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn acquire_guard(&self, event: LifecycleEventType) -> Result<PlatformGuard>;
}
```

`PlatformGuard` is RAII-friendly (fd on Linux, assertion on macOS, shutdown block reason on Windows) but the controller explicitly drops it after sync/timeout for clear logging.

## Python API

```python
from system_lifecycle import LifecycleManager

manager = LifecycleManager(max_wait_seconds=15)
manager.start(on_system_event)
# ... application runs ...
manager.synccomplete()  # returns True if a wait was active
manager.stop()
```

Events expose `event.type` (`shutdown`, `reboot`, `sleep`, `hibernate`) and `event.timestamp` (UTC epoch seconds).

## Testing Strategy

| Tier | Scope | Runs in CI |
|------|-------|------------|
| Unit | State machine, mock platform | Yes |
| Integration | Rust ↔ Python callback/synccomplete | Yes (mock platform) |
| Real OS | Manual shutdown/sleep in VM | No (documented) |

`MockLifecycle` injects synthetic events without touching the OS.

## Build & Packaging

- **Development:** `maturin develop`
- **Production:** `maturin build --release` → wheel with native `.so`/`.dylib`/`.pyd`
- End users install wheels; **no Rust toolchain required at runtime**.

## Security

- No shell commands, no `sudo`, no modification of system shutdown configuration.
- Logs contain lifecycle metadata only (platform, event type, state transitions).

## Platform Honesty

Each OS provides different delay/veto semantics. See `docs/platform-support.md` for per-platform capabilities and limits.
