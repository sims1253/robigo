#!/usr/bin/env bash
# The Linux nightly binaries are profile-guided: about 13% fewer instructions
# across the benchmarks (docs/ledger/pgo.md).
#   scripts/pgo-build.sh profile [TARGET]   # writes .robigo/pgo/robigo.profdata
#   scripts/pgo-build.sh use FILE [TARGET]  # builds with that profile
# `profile` needs GNU R 4.5.0's library: trained on the vendored base alone the
# gain falls to 4% and some benchmarks get slower, because loading packages is
# much of what robigo does. CI profiles in the oracle's container and builds
# on an older system. llvm-profdata comes from `rustup component add
# llvm-tools`, or from PATH with rustc's LLVM major version.
# Pull request perf gates stay without PGO: a profile that moves with every
# commit would swamp their 0.5% threshold.
set -euo pipefail
cd "$(dirname "$0")/.."
host=$(rustc -vV | sed -n 's/^host: //p')

case "${1:-}" in
  profile)
    target=${2:-$host}
    work=${PGO_DIR:-$PWD/.robigo/pgo}
    profdata=$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-profdata
    [ -x "$profdata" ] || profdata=$(command -v llvm-profdata)
    rm -rf "$work/raw" "$work/robigo.profdata"
    mkdir -p "$work/raw"
    RUSTFLAGS="-Cprofile-generate=$work/raw" CARGO_TARGET_DIR="$work/target" \
      cargo build --release --locked -p robigo-cli --target "$target"
    python3 scripts/perf.py fetch
    python3 scripts/perf.py run --metric time --runs 1 --runtime robigo --jobs "$(nproc)" --timeout 300 --quiet \
      --robigo "$work/target/$target/release/robigo" --out "$work/training.json"
    "$profdata" merge -o "$work/robigo.profdata" "$work"/raw/*.profraw
    ;;
  use)
    target=${3:-$host}
    RUSTFLAGS="-Cprofile-use=$(realpath "$2")" cargo build --release --locked -p robigo-cli --target "$target"
    ;;
  *)
    sed -n '2,12p' "$0" >&2
    exit 2
    ;;
esac
