#!/usr/bin/env bash
# robigo-rt's R_HOME tree with the lazy-load databases (L5's boot, the
# switch ROBIGO_RT_BOOT=rdb): scripts/rt-home.sh's tree, then the steps of
# GNU R's build (src/library/Makefile.in's R target) that install base and
# the compiler package, in their order, each run by the robigo-rt that
# boots from the tree (its R_EXE; the tree's base the source until base's
# database is written, ROBIGO_RT_BOOT=stage0, then the database):
#   translations-all     the message catalogues and DESCRIPTION
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
# The tree is rebuilt when the source tree, this script or the binary
# changes (the stamp holds the three).
#   scripts/rt-home-db.sh --robigo-rt BIN [DIR]   # default .robigo/rt-home-db
# RT_HOME_TREE names scripts/rt-home.sh's tree to copy (default its own,
# .robigo/rt-home): a tool that builds both trees outside the checkout
# (scripts/rt-startup.py) passes the one it built.
# BIN is required (never a default target's binary, which another build may
# have written): the tree records what built it in .rt-home-db-builder
# (scripts/rt-builder.sh: path, sha256, build ID, commit), which
# scripts/rt-rdb-compare.sh reports.
set -euo pipefail
cd "$(dirname "$0")/.."
usage() { echo "usage: scripts/rt-home-db.sh --robigo-rt BIN [DIR]" >&2; exit 2; }
[ "${1:-}" = --robigo-rt ] && [ $# -ge 2 ] && [ -n "$2" ] || usage
bin=$2
shift 2
[ $# -le 1 ] || usage
[ -x "$bin" ] || { echo "rt-home-db.sh: no robigo-rt at $bin (cargo build --release -p robigo-rtapi)" >&2; exit 2; }
# (it must carry its commit: checked before anything is built)
scripts/rt-builder.sh "$bin" > /dev/null
out=${1:-.robigo/rt-home-db}
# (no trailing slash or /.: the lock is the tree's sibling, whatever the
# spelling)
while [ "$out" != / ] && { [ "${out%/}" != "$out" ] || [ "${out%/.}" != "$out" ]; }; do
  out=${out%/}
  out=${out%/.}
done
[ -n "$out" ] || out=/
# not scripts/rt-home.sh's tree, which this one is copied from (and whose
# lock rt-home.sh takes below: the same file would deadlock)
if [ "$(realpath -m "$out")" = "$(realpath -m "${RT_HOME_TREE:-.robigo/rt-home}")" ]; then
  echo "rt-home-db.sh: $out is scripts/rt-home.sh's tree, the source of this one" >&2
  exit 2
fi
# one build of the tree at a time (another's rm and mv would interleave
# with this one's), as scripts/build.sh locks its target (the mkdir is
# the mktemp's below too)
mkdir -p "$(dirname "$out")"
exec 9> "$out.lock"
flock 9
src_tree=$(scripts/rt-home.sh ${RT_HOME_TREE:+"$RT_HOME_TREE"})
# the source tree held while it is read (its stamp, the copy below): a
# shared lock on rt-home.sh's, which a rebuild of it takes exclusively
exec 8> "$src_tree.lock"
flock -s 8
rsrc=$(scripts/r-src.sh)
stamp="$(cat "$src_tree/.rt-home") $(sha256sum "$0" | cut -c1-16) $(sha256sum "$bin" | cut -c1-16)"
# (fresh only with its translation cache beside it: a tree whose cache was
# lost, by hand or between the two moves below, is built again)
# and with the files a start reads (base's loader, each database and its
# index): a tree that lost one, by hand, is built again (checked here, under
# the lock, not by the callers)
if [ -f "$out/.rt-home-db" ] && [ "$(cat "$out/.rt-home-db")" = "$stamp" ] &&
   [ -s "$out-cache/base.rtc" ] && [ -s "$out-cache/compiler.rtc" ] &&
   [ -s "$out/library/base/R/base" ] && [ -s "$out/library/base/R/base.rdb" ] &&
   [ -s "$out/library/base/R/base.rdx" ] && [ -s "$out/library/compiler/R/compiler.rdb" ] &&
   [ -s "$out/library/compiler/R/compiler.rdx" ]; then
  echo "$out"
  exit 0
fi
tmp=$(realpath "$(mktemp -d "$(dirname "$out")/.rt-home-db.XXXXXX")")
trap 'rm -rf "$tmp"' EXIT
# the install layout robigo-rt finds its R_HOME by: bin/robigo-rt, lib/robigo
mkdir -p "$tmp/bin" "$tmp/lib/robigo"
# (the tree's contents into a fresh directory: a symlinked tree is read,
# never written)
cp -a "$src_tree/." "$tmp/lib/robigo/"
# (copied: the source tree's lock released)
exec 8>&-
rm -f "$tmp/lib/robigo/.rt-home"
cp "$bin" "$tmp/bin/robigo-rt"
# (the copy is what builds the tree: the binary the stamp names, or the
# build stops)
[ "$(sha256sum "$tmp/bin/robigo-rt" | cut -c1-16)" = "${stamp##* }" ] ||
  { echo "rt-home-db.sh: $bin changed while the tree was being built" >&2; exit 1; }
lib=$tmp/lib/robigo/library
version=4.6.2
# each package's configured DESCRIPTION (DESCRIPTION.in, config.status's),
# the build directory's that mkdesc reads
for p in base tools compiler; do
  mkdir -p "$tmp/src/$p"
  cp "$lib/$p/DESCRIPTION" "$tmp/src/$p/DESCRIPTION"
done
# R_EXE: `[VAR=VALUE...] bin/R --vanilla --no-echo`, the script on stdin,
# the boot the tree's base (`rexe BOOT VAR=VALUE... < script`). A path
# the script needs comes in RT_OUT (Sys.getenv), never inside the R text. HOME is an empty
# directory, as in the oracle image's build: the system profile's
# .libPaths() (whose environment base.rdb holds) adds R_LIBS_USER's
# directory when it exists, which on a developer's machine it may
mkdir -p "$tmp/home"
rexe() {
  local boot=$1
  shift
  # (the translation cache's controls too: a caller's would make every
  # step's session a writer; the two cache steps set their own after)
  env -u R_HOME -u R_LIBS -u R_LIBS_USER -u R_LIBS_SITE \
    -u ROBIGO_TRANSLATION_CACHE -u ROBIGO_TRANSLATION_CACHE_WRITE -u ROBIGO_TRANSLATION_CACHE_STATS \
    HOME="$tmp/home" ROBIGO_RT_BOOT="$boot" "$@" "$tmp/bin/robigo-rt" --vanilla --no-echo > /dev/null
}
# R: src/library/translations/Makefile.in all (its mkdesc runs no R: the
# Makefile sets no R_EXE): the message catalogues (inst/*) and the
# configured DESCRIPTION; base's .popath names the directory when it is
# there as makebasedb.R runs
mkdir -p "$lib/translations"
cp -R "$rsrc/src/library/translations/inst/." "$lib/translations/"
sed -e "s/@VERSION@/$version/g" "$rsrc/src/library/translations/DESCRIPTION.in" > "$lib/translations/DESCRIPTION"
# R: share/make/basepkg.mk mkdesc2 (base-all and tools-all; PKG_BUILT_STAMP
# empty, R_OSTYPE unix)
for p in base tools; do
  echo "Built: R $version; ; $(TZ=UTC date); unix" >> "$lib/$p/DESCRIPTION"
done
# R: src/library/tools/Makefile.in all (its last command; tools' R_EXE is
# `R_ENABLE_JIT=0 TZ=UTC bin/R --vanilla --no-echo`: no compiler yet)
(cd "$tmp/src/tools" && echo "tools:::.install_package_description('.', Sys.getenv('RT_OUT'))" |
  rexe stage0 RT_OUT="$lib/tools" R_DEFAULT_PACKAGES=NULL R_ENABLE_JIT=0 TZ=UTC)
# R: src/library/compiler/Makefile.in (R_EXE = R_ENABLE_JIT=0 TZ=UTC bin/R
# --vanilla --no-echo): all's mkdesc (share/make/basepkg.mk) then
# mklazycomp (share/make/lazycomp.mk; mkR1's all.R is rt-home.sh's
# library/compiler/R/compiler already)
(cd "$tmp/src/compiler" && echo "tools:::.install_package_description('.', Sys.getenv('RT_OUT'))" |
  rexe stage0 RT_OUT="$lib/compiler" R_DEFAULT_PACKAGES=NULL R_ENABLE_JIT=0 TZ=UTC)
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
# R: src/library/Makefile.in R: base's mkdesc (base's R_EXE, `TZ=UTC
# bin/R --vanilla --no-echo`), then the descriptions and the parsed
# NAMESPACE files as RDS (its own, `bin/R --vanilla --no-echo`)
(cd "$tmp/src/base" && echo "tools:::.install_package_description('.', Sys.getenv('RT_OUT'))" |
  rexe rdb RT_OUT="$lib/base" R_DEFAULT_PACKAGES=NULL TZ=UTC)
echo 'tools:::.vinstall_package_descriptions_as_RDS(Sys.getenv("RT_OUT"), "base tools compiler")' |
  rexe rdb RT_OUT="$lib" R_DEFAULT_PACKAGES=tools LC_ALL=C
rm -f "$lib/tools/Meta/nsInfo.rds"
echo 'tools:::.vinstall_package_namespaces_as_RDS(Sys.getenv("RT_OUT"), "base tools compiler")' |
  rexe rdb RT_OUT="$lib" R_DEFAULT_PACKAGES=tools LC_ALL=C
# where the tree was built, which its databases record (.Library, the
# namespaces' path) as GNU R's record its build tree: for
# scripts/rt-rdb-compare.sh's path mapping
echo "$tmp/lib/robigo" > "$tmp/lib/robigo/.rt-home-db-built-at"
# what built it (scripts/rt-builder.sh), for scripts/rt-rdb-compare.sh
scripts/rt-builder.sh "$tmp/bin/robigo-rt" "$(realpath "$bin")" > "$tmp/lib/robigo/.rt-home-db-builder"
# robigo-rt's translation cache (C1, crates/robigo-rt/src/tcache.rs): every
# closure of base's and the compiler's databases translated by this binary,
# and the promise codes this start forces, written beside the tree, never
# inside R_HOME: <prefix>/lib/robigo-cache, R_HOME's path with "-cache"
# (.robigo/rt-home-db-cache for this checkout's tree), so the .rdb files
# stay GNU R's bytes. Then its verify pass: each entry against a fresh
# translation, after the stamp a start checks the cache against is
# written; a difference fails the tree. A start never writes the cache. The
# writer takes the place and the stamp from the tree (R_HOME, its
# .rt-home-db), so the stamp is written first; a failure removes the tree.
cache=$tmp/lib/robigo-cache
mkdir -p "$cache"
echo "$stamp" > "$tmp/lib/robigo/.rt-home-db"
force='for (p in c("base", "compiler")) { ns <- asNamespace(p); for (n in ls(ns, all.names = TRUE)) tryCatch(get(n, envir = ns), error = function(e) NULL) }'
echo "$force" | rexe rdb ROBIGO_TRANSLATION_CACHE_WRITE=1 R_DEFAULT_PACKAGES=NULL ||
  { echo "rt-home-db.sh: writing the translation cache failed" >&2; exit 1; }
for f in base.rtc compiler.rtc; do
  [ -s "$cache/$f" ] || { echo "rt-home-db.sh: the translation cache has no $f" >&2; exit 1; }
done
echo "$force" | rexe rdb ROBIGO_TRANSLATION_CACHE=verify R_DEFAULT_PACKAGES=NULL ||
  { echo "rt-home-db.sh: the translation cache differs from fresh translations" >&2; exit 1; }
rm -rf "$out" "$out-cache"
mv "$tmp/lib/robigo" "$out"
mv "$cache" "$out-cache"
echo "$out"
