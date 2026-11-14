#!/usr/bin/env bash
# The M1 decision bars' measurements of one robigo-rt binary
# (docs/ledger/g3.md, "M1 decision bars"): instructions and estimated cycles
# per iteration of the CALLS and SUITE rows of benchmarks/rt/calls-rows.toml
# (scripts/idiom-vs-r.py --rows --cycles, callgrind N against 2N), run as four
# groups in parallel (the machine limit: at most 4 concurrent callgrind runs),
# then allocations per iteration (--allocs). Raw JSON and the tables go to
# OUTDIR: fib, bt (binarytrees), plain (plain, defaults, named, env-fn), m1
# (superassign, s3 and ROWS), allocs.
#   scripts/rt-bars.sh BIN OUTDIR [--rows REGEX] [--robigo-env K=V ...]
# --rows REGEX adds rows to the m1 group and the allocation run (D2s's
# d2s-* rows, for instance). The bars run every table twice: call it twice.
set -euo pipefail
[ $# -ge 2 ] || { echo "usage: scripts/rt-bars.sh BIN OUTDIR [--rows REGEX] [--robigo-env K=V ...]" >&2; exit 2; }
bin=$1 out=$2
shift 2
extra=
if [ "${1:-}" = --rows ]; then extra="|$2"; shift 2; fi
cd "$(dirname "$0")/.."
mkdir -p "$out"
groups=('^fib$' '^binarytrees$' '^(plain|defaults|named|env-fn)$' "^(superassign|s3$extra)\$")
names=(fib bt plain m1)
pids=()
for i in 0 1 2 3; do
  python3 scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --filter "${groups[$i]}" \
    --robigo "$bin" --no-r --cycles --json "$out/${names[$i]}.json" "$@" \
    > "$out/${names[$i]}.md" 2> "$out/${names[$i]}.err" &
  pids+=($!)
done
st=0
for p in "${pids[@]}"; do wait "$p" || st=1; done
python3 scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --allocs \
  --filter "^(plain|defaults|named|env-fn|superassign|s3|fib|binarytrees$extra)\$" \
  --robigo "$bin" --json "$out/allocs.json" "$@" > "$out/allocs.md" 2> "$out/allocs.err" || st=1
[ $st = 0 ] || { echo "rt-bars: a run failed; see $out/*.err" >&2; exit 1; }
cat "$out"/{fib,bt,plain,m1,allocs}.md
