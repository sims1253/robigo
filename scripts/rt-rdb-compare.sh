#!/usr/bin/env bash
# The lazy-load databases robigo-rt made in the base build
# (scripts/rt-base-build.sh's OUT, default .robigo/rt-base-build) against
# the ones GNU R 4.6.2's own build made (the oracle image's library), record
# by record (scripts/rt-rdb-compare.R: the maps, then each record by key,
# decompressed, then each differing record's class). Runs in the oracle
# image robigo-oracle:4.6.2 (scripts/oracle-run.sh); not in CI.
#   scripts/rt-rdb-compare.sh [OUT] [PKG/NAME ...]
# OUT is scripts/rt-base-build.sh's or a tree of scripts/rt-home-db.sh's
# (.robigo/rt-home-db).
# Build paths differ by construction: our source tree (the pinned R
# sources) and install tree are mapped to GNU R's build tree (the oracle
# built in place; its tools records name the directory, which is not its
# installed R_HOME), so a record that differs only in them is its own
# class.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
# (the default as rt-base-build.sh's, under the checkout; a given OUT is
# the caller's path)
out=${1:-$root/.robigo/rt-base-build}
[ $# -gt 0 ] && shift
[ -d "$out" ] || { echo "error: $out does not exist (run scripts/rt-base-build.sh)" >&2; exit 2; }
out=$(cd "$out" && pwd)
case "$out/" in
  "$root"/*) ;;
  *) echo "error: $out is not under the checkout, which is all the oracle container mounts (scripts/oracle-run.sh)" >&2; exit 2 ;;
esac
# OUT is rt-base-build.sh's (OUT/build/library, installed as
# OUT/inst/lib/robigo) or a tree of scripts/rt-home-db.sh's (OUT/library,
# built at the path its .rt-home-db-built-at holds)
if [ -f "$out/.rt-home-db-built-at" ]; then
  lib=$out/library
  inst=$(cat "$out/.rt-home-db-built-at")
  # (one absolute path, or the map would rewrite nothing or everything)
  case "$inst" in
    /*) [ "$(awk 'END { print NR }' "$out/.rt-home-db-built-at")" = 1 ] ||
          { echo "error: $out/.rt-home-db-built-at holds more than one path" >&2; exit 2; } ;;
    *) echo "error: $out/.rt-home-db-built-at holds no absolute path ('$inst'): rebuild the tree with scripts/rt-home-db.sh" >&2; exit 2 ;;
  esac
  inst2=$inst
  builder=$out/.rt-home-db-builder
else
  [ -d "$out/build/library" ] || { echo "error: $out/build/library does not exist (run scripts/rt-base-build.sh)" >&2; exit 2; }
  lib=$out/build/library
  # (its R_HOME is a link to the build tree: the records name either)
  inst=$out/inst/lib/robigo
  inst2=$out/build
  builder=$out/builder
fi
# what built the databases (scripts/rt-builder.sh's record), in the
# report; a tree that does not say is not compared
# (each field once and whole: a path, a 64-digit sha256, a build ID in
# hex or none, a 40-digit commit; POSIX extended expressions)
for k in 'path: /.*' 'sha256: [0-9a-f]{64}' 'build-id: ([0-9a-f]{2,}|none)' 'commit: [0-9a-f]{40}'; do
  [ "$(grep -c "^${k%%:*}:" "$builder" 2>/dev/null)" = 1 ] && grep -Eqx "$k" "$builder" ||
    { echo "error: $out does not record the robigo-rt that built it ($builder, its ${k%%:*}): rebuild it with scripts/rt-home-db.sh or scripts/rt-base-build.sh --robigo-rt BIN" >&2; exit 2; }
done
# (records, not newlines: an unterminated last line counts)
[ "$(awk 'END { print NR }' "$builder")" = 4 ] ||
  { echo "error: $builder holds more than the robigo-rt's four fields" >&2; exit 2; }
echo "== built by"
sed 's/^/  /' "$builder"
# (after the checks: it may download the tarball)
src=$("$root/scripts/r-src.sh")
cd "$root"
# (the paths go to the container's sh as arguments)
# shellcheck disable=SC2016 # the inner sh expands them
exec scripts/oracle-run.sh 4.6.2 sh -c 'set -eu; h=$(R RHOME); lib=$1; src=$2; inst=$3; inst2=$4; shift 4
  top=$(Rscript --vanilla -e "cat(tools:::.R_top_srcdir_default)")
  [ -n "$h" ] && [ -n "$top" ] || { echo "error: no R_HOME or build tree from the oracle" >&2; exit 2; }
  exec Rscript --vanilla scripts/rt-rdb-compare.R --map "$inst=$top" --map "$inst2=$top" --map "$src=$top" "$lib" "$h/library" "$@"' \
  sh "$lib" "$src" "$inst" "$inst2" "$@"
