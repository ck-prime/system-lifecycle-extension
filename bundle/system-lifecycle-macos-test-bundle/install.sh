#!/usr/bin/env bash
# Create a local venv and install the bundle wheel (Homebrew Python blocks
# system-wide pip install).
set -euo pipefail

BUNDLE_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$BUNDLE_DIR"

WHEEL="$(ls -1 wheels/system_lifecycle-*-cp314-*.whl 2>/dev/null | head -n 1 || true)"
if [[ -z "$WHEEL" ]]; then
  echo "No cp314 wheel found in $BUNDLE_DIR/wheels"
  echo "This bundle is for CPython 3.14. Use system-lifecycle-macos-py310-test-bundle for 3.10."
  exit 1
fi

BASE_PYTHON="${PYTHON:-}"
if [[ -z "$BASE_PYTHON" ]]; then
  if command -v python3.14 >/dev/null 2>&1; then
    BASE_PYTHON="$(command -v python3.14)"
  else
    BASE_PYTHON="$(command -v python3)"
  fi
fi

"$BASE_PYTHON" -m venv "$BUNDLE_DIR/.venv"
"$BUNDLE_DIR/.venv/bin/python" -m pip install --upgrade pip
"$BUNDLE_DIR/.venv/bin/python" -m pip install --force-reinstall "$WHEEL"

echo "Installed $WHEEL into $BUNDLE_DIR/.venv"
echo "Next: ./run_test_detached.sh"
