#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export PYO3_USE_ABI3_FORWARD_COMPATIBILITY="${PYO3_USE_ABI3_FORWARD_COMPATIBILITY:-1}"
export PYO3_PYTHON="${PYO3_PYTHON:-$(command -v python3.10 || command -v python3)}"

PYTHON="${PYTHON:-python3.10}"
if ! command -v "$PYTHON" >/dev/null 2>&1; then
  PYTHON=python3
fi

"$PYTHON" -m maturin develop --release
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
"$PYTHON" -m maturin build --release --out dist

BUNDLE_DIR="$ROOT/bundle/system-lifecycle-linux-py310-test-bundle"
WHEEL="$(ls -1 "$ROOT"/dist/system_lifecycle-*-cp310-*linux*.whl 2>/dev/null | head -n 1 || true)"
if [[ -n "$WHEEL" ]]; then
  mkdir -p "$BUNDLE_DIR/wheels"
  cp -f "$WHEEL" "$BUNDLE_DIR/wheels/"
  echo "Copied wheel to $BUNDLE_DIR/wheels/"
fi

echo "Linux build complete. Wheels are in dist/"
