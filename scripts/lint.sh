#!/usr/bin/env bash
# What CI's lint job checks: clippy on every target of every crate, with the
# features CI builds, warnings being errors. Cargo.toml lists the allowed lints.
# Then the rules no lint has: no safe public function of the runtime crates
# takes a raw Sexp (scripts/check-sexp-safety.py, #736), and no map/set keys
# by raw Sexp or object address without a justification (scripts/check-address-keys.py).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-sexp-safety.py
python3 scripts/check-address-keys.py
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
