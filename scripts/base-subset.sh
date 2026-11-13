#!/usr/bin/env bash
# robigo-rt's base subset (crates/robigo-rt/proto/base-subset.txt): the base
# closures, primitives and .Internals GNU R 4.6.2 reaches running M1's
# scripts (docs/g3.md §14.2 M1), traced on the 4.6.2 oracle image; and the
# bound robigo-rt's loader binds (proto/base-bound.txt: that and the static
# bound).
#   scripts/base-subset.sh           # regenerate the files
#   scripts/base-subset.sh --check   # regenerate into .robigo/ and diff
# Needs docker's robigo-oracle:4.6.2 (scripts/build-oracle.sh --image 4.6.2)
# and the pinned tarball (scripts/r-src.sh). The work happens in
# scripts/base-subset.R, the tracing in scripts/base-subset-trace.R.
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-}" in
  "" | --check) ;;
  *) echo "usage: scripts/base-subset.sh [--check]" >&2; exit 2 ;;
esac
file=crates/robigo-rt/proto/base-subset.txt
bound=crates/robigo-rt/proto/base-bound.txt
src=$(scripts/r-src.sh)
# in the checkout, which is all the oracle container sees
work=.robigo/base-subset
rm -rf "$work"
mkdir -p "$work"
cp -R "$src/src/library/base/R" "$work/base-R"
out=$file
bout=$bound
if [ "${1:-}" = --check ]; then
  out=$work/base-subset.txt
  bout=$work/base-bound.txt
fi
scripts/oracle-run.sh 4.6.2 Rscript --vanilla scripts/base-subset.R "$work" "$out" "$bout"
if [ "${1:-}" = --check ]; then
  diff -u "$file" "$out" || { echo "error: $file is not what scripts/base-subset.sh generates" >&2; exit 1; }
  diff -u "$bound" "$bout" || { echo "error: $bound is not what scripts/base-subset.sh generates" >&2; exit 1; }
  echo "$file and $bound are up to date"
fi
