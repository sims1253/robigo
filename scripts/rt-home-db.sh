#!/usr/bin/env bash
# robigo-rt's R_HOME tree with the lazy-load databases (L5's boot, the
# switch ROBIGO_RT_BOOT=rdb): scripts/rt-home.sh's tree, then the steps of
# GNU R's build (src/library/Makefile.in's R target) that install base and
# the compiler package, in their order, each run by the robigo-rt that
# boots from the tree (its R_EXE; the tree's base the source until base's
# database is written, ROBIGO_RT_BOOT=stage0, then the database):
#   base-all, tools-all  mkdesc2: DESCRIPTION with the shell's Built line;
#                        tools' description installed by R
#   compiler-all         mkdesc (DESCRIPTION, Meta/package.rds), then
#                        mklazycomp: tools:::makeLazyLoading("compiler"),
#                        the closures written as byte code (compiler.rdb,
#                        .rdx; R/compiler the namespace loader)
#   base-mklazycomp      makebasedb.R (base.rdb, .rdx), baseloader.R
#                        installed as library/base/R/base
#   base-mkdesc          base's description installed by R
#   vdesc, vns           Meta/package.rds and Meta/nsInfo.rds of base,
#                        tools and compiler
# Not run: tools-mklazycomp (tools stays source, loadNamespace's
# sys.source), the R target's bibliographies and dictionaries (utils).
# INTERIM(B, datetime.c): the steps that install a description by R
# (`Built:` is format(Sys.time(), ..., tz = "UTC", usetz = TRUE)) read
# scripts/rt-home-db-interim.R first, stand-ins for as.POSIXlt.POSIXct
# and format.POSIXlt in UTC; delete it when B's datetime.c port merges.
# The tree is rebuilt when the source tree, this script, the stand-ins or
# the binary changes (the stamp holds the four).
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
# (no trailing slash: the lock is the tree's sibling, whatever the spelling)
while [ "$out" != / ] && [ "${out%/}" != "$out" ]; do out=${out%/}; done
# one build of the tree at a time (another's rm and mv would interleave
# with this one's), as scripts/build.sh locks its target
mkdir -p "$(dirname "$out")"
exec 9> "$out.lock"
flock 9
src_tree=$(scripts/rt-home.sh)
rsrc=$(scripts/r-src.sh)
interim=$PWD/scripts/rt-home-db-interim.R
stamp="$(cat "$src_tree/.rt-home") $(sha256sum "$0" | cut -c1-16) $(sha256sum "$interim" | cut -c1-16) $(sha256sum "$bin" | cut -c1-16)"
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
lib=$tmp/lib/robigo/library
version=4.6.2
# each package's configured DESCRIPTION (DESCRIPTION.in, config.status's),
# the build directory's that mkdesc reads
for p in base tools compiler; do
  mkdir -p "$tmp/src/$p"
  cp "$lib/$p/DESCRIPTION" "$tmp/src/$p/DESCRIPTION"
done
# R_EXE: `[VAR=VALUE...] bin/R --vanilla --no-echo`, the script on stdin,
# the boot the tree's base (`rexe BOOT VAR=VALUE... < script`); with
# ROBIGO_RT_INTERIM=1 the stand-ins read first
rexe() {
  local boot=$1
  shift
  if [ "${ROBIGO_RT_INTERIM:-}" = 1 ]; then
    { cat "$interim"; cat; } | env -u R_HOME -u ROBIGO_RT_INTERIM ROBIGO_RT_BOOT="$boot" "$@" "$tmp/bin/robigo-rt" --vanilla --no-echo > /dev/null
  else
    env -u R_HOME ROBIGO_RT_BOOT="$boot" "$@" "$tmp/bin/robigo-rt" --vanilla --no-echo > /dev/null
  fi
}
# R: share/make/basepkg.mk mkdesc2 (base-all and tools-all; PKG_BUILT_STAMP
# empty, R_OSTYPE unix)
for p in base tools; do
  echo "Built: R $version; ; $(TZ=UTC date); unix" >> "$lib/$p/DESCRIPTION"
done
# R: src/library/tools/Makefile.in all (its last command; tools' R_EXE is
# `bin/R --vanilla --no-echo`)
(cd "$tmp/src/tools" && echo "tools:::.install_package_description('.', '$lib/tools')" |
  ROBIGO_RT_INTERIM=1 rexe stage0 R_DEFAULT_PACKAGES=NULL)
# R: src/library/compiler/Makefile.in (R_EXE = R_ENABLE_JIT=0 TZ=UTC bin/R
# --vanilla --no-echo): all's mkdesc (share/make/basepkg.mk) then
# mklazycomp (share/make/lazycomp.mk; mkR1's all.R is rt-home.sh's
# library/compiler/R/compiler already)
(cd "$tmp/src/compiler" && echo "tools:::.install_package_description('.', '$lib/compiler')" |
  ROBIGO_RT_INTERIM=1 rexe stage0 R_DEFAULT_PACKAGES=NULL R_ENABLE_JIT=0 TZ=UTC)
echo 'tools:::makeLazyLoading("compiler")' |
  rexe stage0 _R_COMPILE_PKGS_=1 R_COMPILER_SUPPRESS_ALL=1 R_DEFAULT_PACKAGES=NULL LC_ALL=C R_ENABLE_JIT=0 TZ=UTC
for f in compiler.rdb compiler.rdx; do
  [ -s "$lib/compiler/R/$f" ] || { echo "rt-home-db.sh: makeLazyLoading wrote no $f" >&2; exit 1; }
done
# R: src/library/base/Makefile.in ($(top_builddir)/library/$(pkg)/R/$(pkg).rdb)
# (R_EXE is `TZ=UTC bin/R --vanilla --no-echo`; the boot is GNU R's,
# R_ReplFile of the source base)
rexe stage0 TZ=UTC _R_COMPILE_PKGS_=1 R_COMPILER_SUPPRESS_ALL=1 R_DEFAULT_PACKAGES=NULL LC_ALL=C \
  < "$rsrc/src/library/base/makebasedb.R"
for f in base.rdb base.rdx; do
  [ -s "$lib/base/R/$f" ] || { echo "rt-home-db.sh: makebasedb.R wrote no $f" >&2; exit 1; }
done
install -m 644 "$rsrc/src/library/base/baseloader.R" "$lib/base/R/base"
# R: src/library/Makefile.in R: base's mkdesc, then the descriptions and
# the parsed NAMESPACE files as RDS (R_EXE = bin/R --vanilla --no-echo)
(cd "$tmp/src/base" && echo "tools:::.install_package_description('.', '$lib/base')" |
  ROBIGO_RT_INTERIM=1 rexe rdb R_DEFAULT_PACKAGES=NULL)
echo "tools:::.vinstall_package_descriptions_as_RDS(\"$lib\", \"base tools compiler\")" |
  rexe rdb R_DEFAULT_PACKAGES=tools LC_ALL=C
rm -f "$lib/tools/Meta/nsInfo.rds"
echo "tools:::.vinstall_package_namespaces_as_RDS(\"$lib\", \"base tools compiler\")" |
  rexe rdb R_DEFAULT_PACKAGES=tools LC_ALL=C
echo "$stamp" > "$tmp/lib/robigo/.rt-home-db"
rm -rf "$out"
mv "$tmp/lib/robigo" "$out"
echo "$out"
