#!/usr/bin/env bash
# Create a local venv with Python 3.10 and install the cp310 wheel.
set -euo pipefail

BUNDLE_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$BUNDLE_DIR"

WHEEL="$(ls -1 wheels/system_lifecycle-*-cp310-*linux*.whl 2>/dev/null | head -n 1 || true)"
if [[ -z "$WHEEL" ]]; then
  echo "No cp310 Linux wheel found in $BUNDLE_DIR/wheels"
  echo "Build one with: scripts/build_linux.sh"
  exit 1
fi

BASE_PYTHON="${PYTHON:-}"
if [[ -z "$BASE_PYTHON" ]]; then
  if command -v python3.10 >/dev/null 2>&1; then
    BASE_PYTHON="$(command -v python3.10)"
  else
    echo "python3.10 not found. Install CPython 3.10 or set PYTHON=/path/to/python3.10"
    exit 1
  fi
fi

PY_TAG="$("$BASE_PYTHON" -c "import sys; print(f'cp{sys.version_info.major}{sys.version_info.minor}')")"
if [[ "$PY_TAG" != "cp310" ]]; then
  echo "This bundle requires CPython 3.10 (got $PY_TAG from $BASE_PYTHON)."
  exit 1
fi

"$BASE_PYTHON" -m venv "$BUNDLE_DIR/.venv"
"$BUNDLE_DIR/.venv/bin/python" -m pip install --upgrade pip
"$BUNDLE_DIR/.venv/bin/python" -m pip install --force-reinstall "$WHEEL"

echo "Installed $WHEEL into $BUNDLE_DIR/.venv"
echo "Next: ./run_test_detached.sh"
