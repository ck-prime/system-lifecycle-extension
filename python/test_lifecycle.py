#!/usr/bin/env python3
"""Simple lifecycle test: 1-minute sync before synccomplete()."""

from __future__ import annotations

import sys
import time
from datetime import datetime, timezone
from pathlib import Path

from system_lifecycle import LifecycleManager

SYNC_WAIT_SECONDS = 60
MAX_WAIT_SECONDS = 90  # must be greater than SYNC_WAIT_SECONDS
LOG_FILE = Path(__file__).resolve().parent / "log.txt"


def ts() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")


def log(message: str) -> None:
    """Write to console and log.txt (flushed immediately for shutdown testing)."""
    line = f"{message}\n"
    print(message, flush=True)
    with LOG_FILE.open("a", encoding="utf-8") as fh:
        fh.write(line)
        fh.flush()


def main() -> int:
    LOG_FILE.write_text(f"=== lifecycle test started, timestamp={ts()} ===\n", encoding="utf-8")

    manager = LifecycleManager(max_wait_seconds=MAX_WAIT_SECONDS)

    def on_event(event) -> None:
        log(f"event detected: type={event.type}, os_timestamp={event.timestamp}, now={ts()}")
        log(f"waiting for {SYNC_WAIT_SECONDS} seconds, timestamp={ts()}")
        time.sleep(SYNC_WAIT_SECONDS)
        log(f"wait complete, timestamp={ts()}")
        completed = manager.synccomplete()
        log(f"synccomplete called, returned={completed}, timestamp={ts()}")
        log("lifecycle guard released — OS may continue shutdown/sleep")

    manager.start(on_event)
    log(f"running, timestamp={ts()}")
    log(f"lifecycle listener started — log file: {LOG_FILE}")
    log("trigger shutdown or sleep manually, then check log.txt after reboot")

    try:
        while True:
            log(f"running, timestamp={ts()}")
            time.sleep(10)
    except KeyboardInterrupt:
        manager.stop()
        log(f"stopped by user, timestamp={ts()}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
