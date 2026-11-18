#!/usr/bin/env bash
# robigo-rt's base subset (crates/robigo-rt/proto/base-subset.txt): the base
# closures, primitives and .Internals GNU R 4.6.2 reaches running M1's
# scripts (docs/g3.md §14.2 M1), traced on the 4.6.2 oracle image; and the
# bound robigo-rt's loader binds (proto/base-bound.txt: that and the static
# bound); and, INTERIM(L5), the wide base robigo-rt's loader binds
# (proto/base-wide.txt: every binding GNU R 4.6.2 makes in base, by the
# form that defines it; scripts/base-wide.R).
#   scripts/base-subset.sh           # regenerate the files
#   scripts/base-subset.sh --check   # regenerate into .robigo/ and diff
# Needs GNU R 4.6.2: the Rscript on PATH if it is that version (inside the
# oracle image, as CI's base-subset job runs), else docker's
# robigo-oracle:4.6.2 (scripts/build-oracle.sh --image 4.6.2); and the
# pinned tarball (scripts/r-src.sh). The work happens in
# scripts/base-subset.R, the tracing in scripts/base-subset-trace.R.
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-}" in
  "" | --check) ;;
  *) echo "usage: scripts/base-subset.sh [--check]" >&2; exit 2 ;;
esac
file=crates/robigo-rt/proto/base-subset.txt
bound=crates/robigo-rt/proto/base-bound.txt
wide=crates/robigo-rt/proto/base-wide.txt
src=$(scripts/r-src.sh)
# in the checkout, which is all the oracle container sees
work=.robigo/base-subset
rm -rf "$work"
mkdir -p "$work"
cp -R "$src/src/library/base/R" "$work/base-R"
# base's R files, its installed loader and the system profile, as the wide
# index names them (paths under the tarball's src/library)
mkdir -p "$work/library/base"
cp -R "$src/src/library/base/R" "$work/library/base/R"
cp "$src/src/library/base/baseloader.R" "$work/library/base/"
cp -R "$src/src/library/profile" "$work/library/profile"
# the compiler package's R code, an input (robigo-rt evaluates it from source)
cp -R "$src/src/library/compiler/R" "$work/compiler-R"
# The decision bars' D2S and E7 rows (benchmarks/rt/calls-rows.toml, groups
# d2s and e7), as scripts: the bars' other rows are calls.R, fib.R and
# binarytrees.R's, which are traced already (docs/ledger/g3.md, "M1 decision
# bars": every row a bar names must run on robigo-rt).
mkdir -p "$work/rows"
python3 - "$work/rows" <<'PY'
import sys, tomllib
sys.path.insert(0, "scripts")
from perf import rows_with_setups
seen = set()
for r in rows_with_setups(tomllib.load(open("benchmarks/rt/calls-rows.toml", "rb"))):
    # (one script per name: a repeated name would drop a row unseen)
    if r["name"] in seen:
        sys.exit(f"benchmarks/rt/calls-rows.toml: two rows are named {r['name']}")
    seen.add(r["name"])
    if r.get("group") in ("d2s", "e7"):
        with open(f"{sys.argv[1]}/{r['name']}.R", "w") as f:
            f.write(f"{r['setup']}\ng <- function() {{ for (i in 1:3) {{ {r['body']} }}; NULL }}\ng()\n")
PY
out=$file
bout=$bound
wout=$wide
if [ "${1:-}" = --check ]; then
  out=$work/base-subset.txt
  bout=$work/base-bound.txt
  wout=$work/base-wide.txt
fi
# robigo-rt's GNU R (`rt` in scripts/oracle-versions)
. scripts/oracle-pins.sh
if command -v Rscript > /dev/null && [ "$(Rscript -e 'cat(format(getRversion()))' 2> /dev/null)" = "$rt" ]; then
  Rscript --vanilla scripts/base-subset.R "$work" "$out" "$bout"
  R_DEFAULT_PACKAGES=NULL Rscript --vanilla scripts/base-wide.R "$work/library" "$wout"
else
  scripts/oracle-run.sh "$rt" Rscript --vanilla scripts/base-subset.R "$work" "$out" "$bout"
  scripts/oracle-run.sh "$rt" env R_DEFAULT_PACKAGES=NULL Rscript --vanilla scripts/base-wide.R "$work/library" "$wout"
fi
# (before --check's diffs: a failing check is when the report helps)
echo "which scripts reach each name: $work/reached-by.txt"
if [ "${1:-}" = --check ]; then
  diff -u "$file" "$out" || { echo "error: $file is not what scripts/base-subset.sh generates" >&2; exit 1; }
  diff -u "$bound" "$bout" || { echo "error: $bound is not what scripts/base-subset.sh generates" >&2; exit 1; }
  diff -u "$wide" "$wout" || { echo "error: $wide is not what scripts/base-subset.sh generates" >&2; exit 1; }
  echo "$file, $bound and $wide are up to date"
fi
