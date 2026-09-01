#!/usr/bin/env bash
set -euo pipefail

export PYO3_USE_ABI3_FORWARD_COMPATIBILITY="${PYO3_USE_ABI3_FORWARD_COMPATIBILITY:-1}"

maturin develop --release
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
maturin build --release

echo "macOS build complete. Wheels are in target/wheels/"
