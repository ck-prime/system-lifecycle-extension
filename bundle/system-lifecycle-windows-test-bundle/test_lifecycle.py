#!/usr/bin/env python3
"""Simple lifecycle test: sync before synccomplete().

For shutdown testing on Windows, use run_test_detached.ps1 or run_test_detached.cmd
so CMD/Cursor are not part of the shutdown-blocking process.
"""

from __future__ import annotations

import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

from system_lifecycle import LifecycleManager

SYNC_WAIT_SECONDS = 6
MAX_WAIT_SECONDS = 90  # must be greater than SYNC_WAIT_SECONDS
LOG_FILE = Path(__file__).resolve().parent / "log.txt"
LOCK_FILE = Path(__file__).resolve().parent / ".lifecycle_test.lock"


def ts() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")


def log(message: str) -> None:
    """Write to console and log.txt (flushed immediately for shutdown testing)."""
    line = f"{message}\n"
    print(message, flush=True)
    with LOG_FILE.open("a", encoding="utf-8") as fh:
        fh.write(line)
        fh.flush()


def acquire_single_instance_lock():
    """Ensure only one test process runs (avoids duplicate shutdown handlers)."""
    if sys.platform != "win32":
        return None
    import msvcrt

    LOCK_FILE.parent.mkdir(parents=True, exist_ok=True)
    handle = LOCK_FILE.open("w")
    try:
        msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
    except OSError:
        print(
            "Another lifecycle test is already running. "
            "Stop it first (Task Manager -> pythonw.exe) or delete "
            f"{LOCK_FILE}",
            file=sys.stderr,
        )
        sys.exit(1)
    handle.write(str(os.getpid()))
    handle.flush()
    return handle


def main() -> int:
    lock_handle = acquire_single_instance_lock()
    LOG_FILE.write_text(f"=== lifecycle test started, timestamp={ts()} ===\n", encoding="utf-8")

    manager = LifecycleManager(max_wait_seconds=MAX_WAIT_SECONDS)
    sync_lock = __import__("threading").Lock()

    def on_event(event) -> None:
        if not sync_lock.acquire(blocking=False):
            log(f"duplicate event ignored: type={event.type}, now={ts()}")
            return
        try:
            log(f"event detected: type={event.type}, os_timestamp={event.timestamp}, now={ts()}")
            log(f"waiting for {SYNC_WAIT_SECONDS} seconds, timestamp={ts()}")
            time.sleep(SYNC_WAIT_SECONDS)
            log(f"wait complete, timestamp={ts()}")
            completed = manager.synccomplete()
            log(f"synccomplete called, returned={completed}, timestamp={ts()}")
            log("lifecycle guard released — OS may continue shutdown/sleep")
            if str(event.type) in ("shutdown", "reboot"):
                time.sleep(0.5)
                os._exit(0)
        finally:
            sync_lock.release()

    manager.start(on_event)
    log(f"running, timestamp={ts()} pid={os.getpid()}")
    log(f"lifecycle listener started — log file: {LOG_FILE}")
    log("trigger shutdown or sleep manually, then check log.txt after reboot")

    try:
        while True:
            log(f"running, timestamp={ts()}")
            time.sleep(10)
    except KeyboardInterrupt:
        manager.stop()
        log(f"stopped by user, timestamp={ts()}")

    if lock_handle is not None:
        lock_handle.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
