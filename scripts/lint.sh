#!/usr/bin/env bash
# What CI's lint job checks: clippy on every target of every crate, with the
# features CI builds, warnings being errors. Cargo.toml lists the allowed lints.
# Then the rule no lint has: no safe public function of the runtime crates
# takes a raw Sexp (scripts/check-sexp-safety.py, #736).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-sexp-safety.py
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
