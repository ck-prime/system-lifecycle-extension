$ErrorActionPreference = "Stop"

$env:PYO3_USE_ABI3_FORWARD_COMPATIBILITY = "1"

maturin develop --release
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
maturin build --release

Write-Host "Windows build complete. Wheels are in target/wheels/"
