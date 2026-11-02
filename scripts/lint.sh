#!/usr/bin/env bash
# What CI's lint job checks: clippy on every target of every crate, with the
# features CI builds, warnings being errors. Cargo.toml lists the allowed lints.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
