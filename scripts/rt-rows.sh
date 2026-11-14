#!/usr/bin/env bash
# measure.sh BIN OUTDIR: the bars' rows (CALLS, SUITE, D2S) under callgrind with
# --cycles, in 4 parallel groups (<= 4 concurrent callgrind runs), then allocations.
set -euo pipefail
BIN=$1; OUT=$2; shift 2
REPO=/home/m0hawk/.cache/x1-wt/harness
mkdir -p "$OUT"
cd "$REPO"
groups=('^fib$' '^binarytrees$' '^(plain|defaults|named|env-fn)$' '^(superassign|s3)$')
names=(fib bt plain m1)
pids=()
for i in 0 1 2 3; do
  python3 scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --filter "${groups[$i]}" \
    --robigo "$BIN" --no-r --cycles --json "$OUT/${names[$i]}.json" "$@" > "$OUT/${names[$i]}.md" 2> "$OUT/${names[$i]}.err" &
  pids+=($!)
done
st=0
for p in "${pids[@]}"; do wait "$p" || st=1; done
python3 scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --allocs \
  --filter '^(plain|defaults|named|env-fn|superassign|s3|fib|binarytrees)$' \
  --robigo "$BIN" --json "$OUT/allocs.json" "$@" > "$OUT/allocs.md" 2> "$OUT/allocs.err" || st=1
exit $st
