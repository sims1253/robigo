#!/usr/bin/env bash
# E3 determinism (docs/g3.md §15.3): experiment A's stress script
# (tests/rt/stress/finalizer-depths.R) run RUNS times per profile on
# robigo-rt, each a fresh process with the system's address-space
# randomization, so every run lays the heap and the C stack out anew. A run
# passes when it misses no finalizer (its output is the recorded one:
# "missed 0 of 625" for both kinds); the script fails if any run does not.
#
#   scripts/rt-stress.sh [--runs N] [--profiles "eval bc"] [BIN]
#
# BIN defaults to $ROBIGO_RT_BIN, then the release build of the target
# directory. ROBIGO_* collector modes in the environment pass through
# (e.g. ROBIGO_GCTORTURE=1). Prints one line per run and a summary.
set -euo pipefail
cd "$(dirname "$0")/.."
runs=30
profiles="eval bc"
while [ $# -gt 0 ]; do
  case "$1" in
    --runs) runs=$2; shift 2 ;;
    --profiles) profiles=$2; shift 2 ;;
    -h|--help) sed -n '2,/^set /{/^#/p}' "$0"; exit 0 ;;
    -*) echo "rt-stress: unknown option $1" >&2; exit 2 ;;
    *) break ;;
  esac
done
target=${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}
bin=${1:-${ROBIGO_RT_BIN:-$target/release/robigo-rt}}
[ -x "$bin" ] || { echo "rt-stress: no robigo-rt at $bin (cargo build --release -p robigo-rtapi)" >&2; exit 2; }
aslr=$(cat /proc/sys/kernel/randomize_va_space 2> /dev/null || echo unknown)
[ "$aslr" = 2 ] || echo "rt-stress: warning: kernel.randomize_va_space is $aslr, not 2 (full ASLR)" >&2
case=tests/rt/stress/finalizer-depths.R
want=tests/rt/R-4.6.2/stress/finalizer-depths.Rout
libs=$(mktemp -d)
trap 'rm -rf "$libs"' EXIT
scripts/rt-capi-build.sh "$libs"
export RT_CAPI_LIBS=$libs
echo "rt-stress: $bin, $runs runs per profile ($profiles), ASLR $aslr"
bad=0
for p in $profiles; do
  missed=0
  for ((i = 1; i <= runs; i++)); do
    out=$(ROBIGO_RT_PROFILE=$p "$bin" "$case" 2> /dev/null) || { echo "$p run $i: exit status $?"; bad=$((bad + 1)); continue; }
    if [ "$out" = "$(cat "$want")" ]; then
      echo "$p run $i: ok"
    else
      echo "$p run $i: missed finalizers:"
      printf '%s\n' "$out" | sed 's/^/  /'
      missed=$((missed + 1))
    fi
  done
  echo "rt-stress: $p: $((runs - missed)) of $runs runs missed nothing"
  bad=$((bad + missed))
done
[ "$bad" = 0 ] || { echo "rt-stress: $bad runs failed" >&2; exit 1; }
