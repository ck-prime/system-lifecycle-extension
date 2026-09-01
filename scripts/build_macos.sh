#!/usr/bin/env bash
set -euo pipefail

export PYO3_USE_ABI3_FORWARD_COMPATIBILITY="${PYO3_USE_ABI3_FORWARD_COMPATIBILITY:-1}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_314="$ROOT/bundle/system-lifecycle-macos-test-bundle/wheels"
BUNDLE_310="$ROOT/bundle/system-lifecycle-macos-py310-test-bundle/wheels"

maturin develop --release
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
maturin build --release

copy_tag() {
  local tag="$1"
  local dest="$2"
  mkdir -p "$dest"
  local wheel
  wheel="$(ls -1 "$ROOT"/target/wheels/system_lifecycle-*-"${tag}"-*.whl 2>/dev/null | head -n 1 || true)"
  if [[ -z "$wheel" ]]; then
    echo "warning: no ${tag} wheel in target/wheels (skipping $dest)"
    return 0
  fi
  rm -f "$dest"/system_lifecycle-*.whl
  cp "$wheel" "$dest/"
  echo "Copied $(basename "$wheel") to $dest"
}

copy_tag cp314 "$BUNDLE_314"

if command -v python3.10 >/dev/null 2>&1; then
  maturin build --release -i "$(command -v python3.10)"
  copy_tag cp310 "$BUNDLE_310"
else
  echo "python3.10 not found; skipped cp310 bundle wheel"
fi

echo "macOS build complete. Wheels are in target/wheels/"
