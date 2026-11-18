#!/usr/bin/env bash
# The Linux nightly binaries are profile-guided: about 13% fewer instructions
# across the benchmarks (docs/ledger/pgo.md).
#   scripts/pgo-build.sh profile [TARGET]   # writes .robigo/pgo/robigo.profdata
#   scripts/pgo-build.sh use FILE [TARGET]  # builds with that profile
#   scripts/pgo-build.sh -p robigo-rtapi profile|use ...   # robigo-rt (G3; docs/g3.md §13)
# `profile` needs GNU R 4.5.0's library: trained on the vendored base alone the
# gain falls to 4% and some benchmarks get slower, because loading packages is
# much of what robigo does. CI profiles in the oracle's container and builds
# on an older system. llvm-profdata comes from `rustup component add
# llvm-tools`, or from PATH with rustc's LLVM major version.
# Pull request perf gates stay without PGO: a profile that moves with every
# commit would swamp their 0.5% threshold.
#
# With -p robigo-rtapi the binary is robigo-rt, profiled (into
# .robigo/pgo-rt/robigo-rt.profdata) on what it is measured on:
#   - every row of benchmarks/rt/calls-rows.toml (perf-rt's rows), as
#     scripts/idiom-vs-r.py runs one, its body looped `n` times after its setup;
#   - benchmarks/suite's calls.R, fib.R and binarytrees.R (E1's programs);
#   - the boot (`robigo-rt -e 1`, five times);
#   - tests/rt and its ratcheted tests/r cases (the rt conformance runner,
#     ROBIGO_RT_BIN pointed at the instrumented binary; scripts/rt-home.sh's
#     R_HOME tree first).
# docs/g3.md §13 compares wall time on this shipped build.
set -euo pipefail
cd "$(dirname "$0")/.."
host=$(rustc -vV | sed -n 's/^host: //p')
pkg=robigo-cli
if [ "${1:-}" = -p ]; then
  pkg=${2:?-p needs a package}
  shift 2
fi
case $pkg in
  robigo-cli | robigo-rtapi) ;;
  *) echo "pgo-build.sh: -p robigo-cli or -p robigo-rtapi" >&2; exit 2 ;;
esac

profdata_tool() {
  local t
  t=$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-profdata
  [ -x "$t" ] || t=$(command -v llvm-profdata)
  echo "$t"
}

# robigo-rt's training runs, with the instrumented binary $1
train_rt() {
  local bin=$1 work=$2
  mkdir -p "$work/rows"
  python3 - "$work/rows" <<'PY'
import os, sys, tomllib
sys.path.insert(0, "scripts")
from perf import rows_with_setups
doc = tomllib.load(open("benchmarks/rt/calls-rows.toml", "rb"))
for r in rows_with_setups(doc):
    with open(os.path.join(sys.argv[1], f"{r['name']}.R"), "w") as f:
        f.write(f"{r['setup']}\ng <- function() {{ for (i in 1:{r['n']}) {{ {r['body']} }}; NULL }}\ng()\n")
PY
  # (a row robigo-rt cannot run yet, d0-cbind say, trains what it reaches)
  local f failed=()
  for f in "$work"/rows/*.R benchmarks/suite/calls.R benchmarks/suite/fib.R benchmarks/suite/binarytrees.R; do
    env -u R_HOME "$bin" "$f" > /dev/null 2>&1 || failed+=("$(basename "$f" .R)")
  done
  [ ${#failed[@]} -eq 0 ] || echo "pgo-build.sh: ${#failed[@]} program(s) stop with an error under robigo-rt: ${failed[*]}" >&2
  for _ in 1 2 3 4 5; do env -u R_HOME "$bin" -e 1 > /dev/null; done
  scripts/rt-home.sh > /dev/null
  # the cases' outcome is the conformance gate's business; here they only run
  ROBIGO_RT_BIN=$bin cargo test --release --locked -p robigo-rtapi --test conformance -- --exact rt_conformance \
    > "$work/conformance.log" 2>&1 || echo "pgo-build.sh: the rt conformance run did not pass (see $work/conformance.log)" >&2
}

case "${1:-}" in
  profile)
    target=${2:-$host}
    profdata=$(profdata_tool)
    if [ "$pkg" = robigo-cli ]; then
      work=${PGO_DIR:-$PWD/.robigo/pgo}
      rm -rf "$work/raw" "$work/robigo.profdata"
      mkdir -p "$work/raw"
      RUSTFLAGS="-Cprofile-generate=$work/raw" CARGO_TARGET_DIR="$work/target" \
        cargo build --release --locked -p robigo-cli --target "$target"
      python3 scripts/perf.py fetch
      python3 scripts/perf.py run --metric time --runs 1 --runtime robigo --jobs "$(nproc)" --timeout 300 --quiet \
        --robigo "$work/target/$target/release/robigo" --out "$work/training.json"
      "$profdata" merge -o "$work/robigo.profdata" "$work"/raw/*.profraw
    else
      work=${PGO_DIR:-$PWD/.robigo/pgo-rt}
      rm -rf "$work/raw" "$work/robigo-rt.profdata" "$work/rows"
      mkdir -p "$work/raw"
      # (more value-profile counters per site: the default four run out on
      # the evaluator's indirect calls, and LLVM then says so on stderr)
      RUSTFLAGS="-Cprofile-generate=$work/raw -Cllvm-args=-vp-counters-per-site=8" CARGO_TARGET_DIR="$work/target" \
        cargo build --release --locked -p robigo-rtapi --bin robigo-rt --target "$target"
      train_rt "$work/target/$target/release/robigo-rt" "$work"
      "$profdata" merge -o "$work/robigo-rt.profdata" "$work"/raw/*.profraw
      echo "$work/robigo-rt.profdata"
    fi
    ;;
  use)
    target=${3:-$host}
    if [ "$pkg" = robigo-cli ]; then
      RUSTFLAGS="-Cprofile-use=$(realpath "$2")" cargo build --release --locked -p robigo-cli --target "$target"
    else
      # (rustc's -Cprofile-use warns of no function the profile misses, so a
      # stale profile still builds; regenerate it with the commit it ships)
      RUSTFLAGS="-Cprofile-use=$(realpath "$2")" \
        cargo build --release --locked -p robigo-rtapi --bin robigo-rt --target "$target"
    fi
    ;;
  *)
    sed -n '2,25p' "$0" >&2
    exit 2
    ;;
esac
