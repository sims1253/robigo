#!/usr/bin/env bash
# robigo-rt's R_HOME tree with base's lazy-load database (L5's boot, the
# switch ROBIGO_RT_BOOT=rdb): scripts/rt-home.sh's tree, then base's
# database rule as GNU R's build runs it (src/library/base/Makefile.in's
# `$(pkg).rdb`: makebasedb.R, run by R booted from the tree's source base,
# writes library/base/R/base.rdb and .rdx; then baseloader.R installed as
# library/base/R/base, whose ..lazyLoad binds base from them at start-up).
# The robigo-rt that writes the database is the one that boots from it:
# the tree is rebuilt when the source tree, this script or the binary
# changes (the stamp holds the three).
#   scripts/rt-home-db.sh [--robigo-rt BIN] [DIR]   # default .robigo/rt-home-db
# BIN defaults to cargo's release robigo-rt.
set -euo pipefail
cd "$(dirname "$0")/.."
bin=
if [ "${1:-}" = --robigo-rt ]; then
  [ $# -ge 2 ] || { echo "usage: scripts/rt-home-db.sh [--robigo-rt BIN] [DIR]" >&2; exit 2; }
  bin=$2
  shift 2
fi
if [ -z "$bin" ]; then
  # cargo's target directory, as scripts/build.sh finds it
  target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
  bin=$target/release/robigo-rt
fi
[ -x "$bin" ] || { echo "rt-home-db.sh: no robigo-rt at $bin (cargo build --release -p robigo-rtapi)" >&2; exit 2; }
out=${1:-.robigo/rt-home-db}
# one build of the tree at a time (another's rm and mv would interleave
# with this one's), as scripts/build.sh locks its target
mkdir -p "$(dirname "$out")"
exec 9> "$out.lock"
flock 9
src_tree=$(scripts/rt-home.sh)
rsrc=$(scripts/r-src.sh)
stamp="$(cat "$src_tree/.rt-home") $(sha256sum "$0" | cut -c1-16) $(sha256sum "$bin" | cut -c1-16)"
if [ -f "$out/.rt-home-db" ] && [ "$(cat "$out/.rt-home-db")" = "$stamp" ]; then
  echo "$out"
  exit 0
fi
mkdir -p "$(dirname "$out")"
tmp=$(mktemp -d "$(dirname "$out")/.rt-home-db.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
# the install layout robigo-rt finds its R_HOME by: bin/robigo-rt, lib/robigo
mkdir -p "$tmp/bin" "$tmp/lib/robigo"
# (the tree's contents into a fresh directory: a symlinked tree is read,
# never written)
cp -a "$src_tree/." "$tmp/lib/robigo/"
rm -f "$tmp/lib/robigo/.rt-home"
cp "$bin" "$tmp/bin/robigo-rt"
# R: src/library/base/Makefile.in ($(top_builddir)/library/$(pkg)/R/$(pkg).rdb)
# (R_EXE is `TZ=UTC bin/R --vanilla --no-echo`; the boot is GNU R's,
# R_ReplFile of the source base)
TZ=UTC _R_COMPILE_PKGS_=1 R_COMPILER_SUPPRESS_ALL=1 R_DEFAULT_PACKAGES=NULL LC_ALL=C ROBIGO_RT_BOOT=stage0 \
  env -u R_HOME "$tmp/bin/robigo-rt" --vanilla --no-echo < "$rsrc/src/library/base/makebasedb.R" > /dev/null
for f in base.rdb base.rdx; do
  [ -s "$tmp/lib/robigo/library/base/R/$f" ] || { echo "rt-home-db.sh: makebasedb.R wrote no $f" >&2; exit 1; }
done
install -m 644 "$rsrc/src/library/base/baseloader.R" "$tmp/lib/robigo/library/base/R/base"
echo "$stamp" > "$tmp/lib/robigo/.rt-home-db"
rm -rf "$out"
mv "$tmp/lib/robigo" "$out"
echo "$out"
