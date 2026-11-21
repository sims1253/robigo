#!/usr/bin/env bash
# The lazy-load databases robigo-rt made in the base build
# (scripts/rt-base-build.sh's OUT, default .robigo/rt-base-build) against
# the ones GNU R 4.6.2's own build made (the oracle image's library), record
# by record (scripts/rt-rdb-compare.R: the maps, then each record by key,
# decompressed, then each differing record's class). Runs in the oracle
# image robigo-oracle:4.6.2 (scripts/oracle-run.sh); not in CI.
#   scripts/rt-rdb-compare.sh [OUT] [PKG/NAME ...]
# Build paths differ by construction: our source tree (the pinned R
# sources) and install tree are mapped to GNU R's build tree (the oracle
# built in place; its tools records name the directory, which is not its
# installed R_HOME), so a record that differs only in them is its own
# class.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-.robigo/rt-base-build}
[ $# -gt 0 ] && shift
[ -d "$out" ] || { echo "error: $out does not exist (run scripts/rt-base-build.sh)" >&2; exit 2; }
out=$(cd "$out" && pwd)
src=$("$root/scripts/r-src.sh")
[ -d "$out/build/library" ] || { echo "error: $out/build/library does not exist (run scripts/rt-base-build.sh)" >&2; exit 2; }
cd "$root"
# (the paths go to the container's sh as arguments)
# shellcheck disable=SC2016 # the inner sh expands them
exec scripts/oracle-run.sh 4.6.2 sh -c 'h=$(R RHOME); lib=$1; src=$2; inst=$3; shift 3
  top=$(Rscript -e "cat(tools:::.R_top_srcdir_default)")
  exec Rscript scripts/rt-rdb-compare.R --map "$inst=$top" --map "$src=$top" "$lib" "$h/library" "$@"' \
  sh "$out/build/library" "$src" "$out/inst/lib/robigo" "$@"
