#!/usr/bin/env bash
# The M1 decision bars' measurements of one robigo-rt binary
# (docs/ledger/g3.md, "M1 decision bars"): instructions and estimated cycles
# per iteration of the CALLS and SUITE rows of benchmarks/rt/calls-rows.toml
# (scripts/idiom-vs-r.py --rows --cycles, callgrind N against 2N), run as four
# groups in parallel (the machine limit: at most 4 concurrent callgrind runs),
# then allocations per iteration (--allocs). Raw JSON and the tables go to
# OUTDIR: fib, bt (binarytrees), plain (plain, defaults, named, env-fn), m1
# (superassign, s3 and ROWS), allocs.
#   scripts/rt-bars.sh BIN OUTDIR [--rows REGEX]... [--keep DIR] [--robigo-env K=V ...]
# --rows REGEX (anywhere, repeatable) adds rows to the m1 group and the
# allocation run (D2s's d2s-* rows, for instance); it is this script's, never
# idiom-vs-r.py's (whose --rows is the rows file). --keep DIR keeps the
# callgrind files, one subdirectory per group (the groups run in parallel,
# and their loop scripts would overwrite one another in one directory).
# Other options go to every idiom-vs-r.py run. The bars run every table
# twice: call it twice.
set -euo pipefail
usage() { echo "usage: scripts/rt-bars.sh BIN OUTDIR [--rows REGEX]... [--keep DIR] [--robigo-env K=V ...]" >&2; exit 2; }
[ $# -ge 2 ] || usage
bin=$1 out=$2
shift 2
extra= keep= pass=()
while [ $# -gt 0 ]; do
  case $1 in
    --rows | --keep)
      [ $# -ge 2 ] && [ "${2#--}" = "$2" ] || { echo "rt-bars: $1 needs a value" >&2; usage; }
      if [ "$1" = --rows ]; then extra="$extra|$2"; else keep=$2; fi
      shift 2 ;;
    --rows=* | --keep=*) echo "rt-bars: write $1 as two words" >&2; usage ;;
    *) pass+=("$1"); shift ;;
  esac
done
cd "$(dirname "$0")/.."
mkdir -p "$out"
groups=('^fib$' '^binarytrees$' '^(plain|defaults|named|env-fn)$' "^(superassign|s3$extra)\$")
names=(fib bt plain m1)
keepdir() { [ -z "$keep" ] || printf '%s\n' --keep "$keep/$1"; }
pids=()
for i in 0 1 2 3; do
  mapfile -t k < <(keepdir "${names[$i]}")
  python3 scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --filter "${groups[$i]}" \
    --robigo "$bin" --no-r --cycles --json "$out/${names[$i]}.json" "${k[@]}" "${pass[@]}" \
    > "$out/${names[$i]}.md" 2> "$out/${names[$i]}.err" &
  pids+=($!)
done
st=0
for p in "${pids[@]}"; do wait "$p" || st=1; done
mapfile -t k < <(keepdir allocs)
python3 scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --allocs \
  --filter "^(plain|defaults|named|env-fn|superassign|s3|fib|binarytrees$extra)\$" \
  --robigo "$bin" --json "$out/allocs.json" "${k[@]}" "${pass[@]}" > "$out/allocs.md" 2> "$out/allocs.err" || st=1
[ $st = 0 ] || { echo "rt-bars: a run failed; see $out/*.err" >&2; exit 1; }
cat "$out"/{fib,bt,plain,m1,allocs}.md
