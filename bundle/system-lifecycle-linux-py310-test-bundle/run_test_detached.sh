#!/usr/bin/env bash
# Run the lifecycle test detached from the terminal so this process
# receives logind shutdown/sleep events directly.
set -euo pipefail

BUNDLE_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$BUNDLE_DIR"

if [[ ! -x "$BUNDLE_DIR/.venv/bin/python" ]]; then
  echo "Bundle venv not found. Running install.sh..."
  chmod +x "$BUNDLE_DIR/install.sh"
  "$BUNDLE_DIR/install.sh"
fi

PYTHON="$BUNDLE_DIR/.venv/bin/python"

if ! "$PYTHON" -c "import system_lifecycle" >/dev/null 2>&1; then
  echo "system_lifecycle is not installed in the bundle venv."
  echo "Run: ./install.sh"
  exit 1
fi

nohup "$PYTHON" "$BUNDLE_DIR/test_lifecycle.py" >"$BUNDLE_DIR/test_lifecycle.out" 2>&1 &
PID=$!
echo "Lifecycle test started detached (pid=$PID)."
echo "If a previous test is still running, kill it first."
echo "Logs: $BUNDLE_DIR/log.txt and $BUNDLE_DIR/lifecycle_native.log"
echo "Shutdown test:  systemctl poweroff   (or use your desktop Shut Down)"
echo "Sleep test:     systemctl suspend    (or close the lid)"
echo "Stop without shutting down: kill $PID"
