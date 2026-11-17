#!/bin/bash
# D0c: the builtins GNU R 4.6.2 reaches building and booting base and
# compiler (docs/ledger/g3/2026-11-16-d0-count.md). From the checkout root:
#   docs/ledger/g3/2026-11-16-d0-count/run.sh [--callgrind] [RBUILD [WORK]]
# RBUILD is a `-g` GNU R 4.6.2 build tree (default
# ~/.cache/robigo-research/R-4.6.2-build), only read; WORK (default
# .robigo/d0-count) receives copies of it and the traces. Writes
# WORK/traces (as committed next to this script), WORK/boot-set.tsv and
# WORK/report.txt. --callgrind also profiles every run with callgrind and
# checks that it reaches the same names.c C functions as the gdb trace.
# Needs gdb, python3 and, with --callgrind, valgrind. About 15 minutes for
# the gdb pass, 35 more for callgrind.
set -euo pipefail
cg=0
if [ "${1:-}" = --callgrind ]; then cg=1; shift; fi
RBUILD=$(cd "${1:-$HOME/.cache/robigo-research/R-4.6.2-build}" && pwd)
WORK=${2:-.robigo/d0-count}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(pwd)
mkdir -p "$WORK"
WORK=$(cd "$WORK" && pwd)

# A copy of the build tree with bin/R paths pointing at it, and the tracer.
copy() {
  rm -rf "$1"
  cp -a "$RBUILD" "$1"
  sed -i "s#$RBUILD#$1#g" "$1/bin/R" "$1/Makeconf"
  mv "$1/bin/R" "$1/bin/R.real"
  cp "$HERE/tracer.sh" "$1/bin/R"
  chmod +x "$1/bin/R"
}

srcdir=$(grep -m1 '^top_srcdir' "$RBUILD/src/library/base/Makefile" | sed 's/.*= *//')
python3 "$HERE/funtab.py" "$srcdir/src/main/names.c" "$RBUILD/lib/libR.so" > "$WORK/funtab.tsv"
cut -f2 "$WORK/funtab.tsv" | sort | uniq -c | awk '{print $2, $1}' > "$WORK/funs.txt"

# The build's invocations, traced by gdb.
copy "$WORK/Rb"
D0C_GDB=$HERE/split.gdb.py D0C_FUNS=$WORK/funs.txt "$HERE/build-steps.sh" "$WORK/Rb" "$WORK/log-gdb" gdb
# -e 1 from the databases, in a pristine copy.
copy "$WORK/Re1"
mkdir -p "$WORK/log-e1"
D0C_FUNS=$WORK/funs.txt D0C_SPLIT_OUT=$WORK/log-e1/e1.split R_DEFAULT_PACKAGES=NULL \
  "$WORK/Re1/bin/R.real" -d gdb --debugger-args="-batch -nx -x $HERE/split.gdb.py" \
  --no-echo --no-restore --vanilla -e 1 > "$WORK/log-e1/e1.stdout" 2>&1

rm -rf "$WORK/traces"
mkdir -p "$WORK/traces/build"
cp "$WORK"/log-gdb/*.split "$WORK"/log-gdb/*.env "$WORK/traces/build/"
python3 "$HERE/report.py" --label "$WORK/log-gdb" > "$WORK/traces/build/invocations.tsv"
cp "$WORK/log-e1/e1.split" "$WORK/traces/"

if [ $cg = 1 ]; then
  copy "$WORK/Rcg"
  "$HERE/build-steps.sh" "$WORK/Rcg" "$WORK/log-cg" callgrind
  copy "$WORK/Re1cg"
  R_DEFAULT_PACKAGES=NULL "$WORK/Re1cg/bin/R.real" \
    -d "valgrind --tool=callgrind --callgrind-out-file=$WORK/log-cg/e1.cg" \
    --no-echo --no-restore --vanilla -e 1 > /dev/null 2>&1
  cut -d' ' -f1 "$WORK/funs.txt" | sort > "$WORK/funs.sorted"
  bad=0
  for s in "$WORK"/log-gdb/*.split "$WORK/log-e1/e1.split"; do
    b=$(basename "$s" .split)
    python3 "$HERE/cg-dofuns.py" "$WORK/log-cg/$b.cg" | awk '{print $1}' | sort -u | comm -12 - "$WORK/funs.sorted" > "$WORK/log-cg/$b.fns"
    cut -f3 "$s" | sort -u > "$WORK/log-cg/$b.gdbfns"
    if cmp -s "$WORK/log-cg/$b.fns" "$WORK/log-cg/$b.gdbfns"; then
      echo "callgrind and gdb agree on $b: $(wc -l < "$WORK/log-cg/$b.fns") functions"
    else
      echo "callgrind and gdb DISAGREE on $b:"; diff "$WORK/log-cg/$b.fns" "$WORK/log-cg/$b.gdbfns" || true; bad=1
    fi
  done
  [ $bad = 0 ] || exit 1
fi

# M1's rows, and robigo-rt's other rows, from this checkout.
grep -oE '^\s*(row\(|op!\()"[^"]+"' "$ROOT/crates/robigo-rtbuiltins/src/registry.rs" | sed -E 's/.*"([^"]+)"/\1/' | sort -u > "$WORK/registry.txt"
for f in crates/robigo-rt/src/prims.rs crates/robigo-rtapi/src/dotcode.rs; do
  grep -oE '(PrimDef \{ name: |^\s*(special|builtin|special_fn)\()"[^"]+"' "$ROOT/$f" | sed -E 's/.*"([^"]+)"$/\1/'
done | sort -u > "$WORK/rt-prims.txt"
python3 "$HERE/report.py" "$WORK/funtab.tsv" "$WORK/registry.txt" "$WORK/rt-prims.txt" "$WORK/traces" "$WORK/boot-set.tsv" | tee "$WORK/report.txt"
