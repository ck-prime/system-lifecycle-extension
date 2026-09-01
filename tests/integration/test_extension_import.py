"""Integration tests for the system_lifecycle extension."""

from __future__ import annotations

import pytest

system_lifecycle = pytest.importorskip("system_lifecycle")
LifecycleManager = system_lifecycle.LifecycleManager


def test_lifecycle_manager_constructs() -> None:
    manager = LifecycleManager(max_wait_seconds=5)
    assert manager.is_running() is False
