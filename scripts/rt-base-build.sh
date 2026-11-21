#!/usr/bin/env bash
# GNU R 4.6.2's own build of its library's base packages, run by robigo-rt
# (L4, m2m3-plan): src/library/Makefile.in's R target, its steps for
# profile, translations, base, tools and compiler, with robigo-rt as the R
# the Makefiles call. The package Makefiles and share/make/*.mk run as they
# are; what configure would substitute into them is substituted here, and
# only what they read.
#   scripts/rt-base-build.sh --robigo-rt BIN [--shim FILE] [OUT]   # default OUT .robigo/rt-base-build
# BIN is required (never a default target's binary, which another build may
# have written); OUT/builder records it (scripts/rt-builder.sh), which
# scripts/rt-rdb-compare.sh reports.
# The R target's steps, in its order (src/library/Makefile.in), each run
# whatever the step before it did, so that each reports what stops it:
#   profile-all translations-all base-all tools-all compiler-all
#   base-mklazycomp tools-mklazycomp base-mkdesc vdesc vns
# (then the R target installs bibliographies and dictionaries, which need
# utils: not run). Each step's output is in OUT/logs/STEP.log; the report,
# OUT/report.txt, gives each step's status and the first error it printed.
#
# The build tree (OUT/build, configure's top_builddir) has:
#   Makeconf            the variables the R target, the package Makefiles and
#                       basepkg.mk/lazycomp.mk read, as configure sets them on
#                       the oracle image (R_PKGS_BASE is base tools compiler,
#                       R_PKGS_BASE1 empty: the other packages are not built)
#   src/library/*/{Makefile,DESCRIPTION}  from their .in files (config.status's
#                       substitutions), newer than a stub config.status, so
#                       make does not remake them
#   bin/R               execs robigo-rt (OUT/inst/bin/robigo-rt, whose R_HOME
#                       is OUT/inst/lib/robigo, the build tree: the install
#                       layout robigo-rt finds its tree by), the script on
#                       stdin as the Makefiles pipe it; with --shim, FILE is
#                       read first (stand-ins for internals robigo-rt lacks,
#                       each naming its owner)
#   library/tools/libs/tools.so  tools' native code from scripts/rt-home.sh
#                       (the build tree has no src/library/tools/src, so
#                       tools' mksrc builds nothing)
# The tree is made fresh each run; robigo-rt's binary and the tarball are
# what it depends on (the stamp in OUT/stamp names both).
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
bin=
shim=
while [ $# -gt 0 ]; do
  case "$1" in
    --robigo-rt) bin=${2:-}; shift $(( $# >= 2 ? 2 : 1 )) ;;
    --shim) shim=$(realpath "$2"); shift 2 ;;
    -*) echo "rt-base-build.sh: unknown option $1" >&2; exit 2 ;;
    *) break ;;
  esac
done
out=${1:-.robigo/rt-base-build}
case "$out" in /*) ;; *) out=$root/$out ;; esac
[ -n "$bin" ] || { echo "usage: scripts/rt-base-build.sh --robigo-rt BIN [--shim FILE] [OUT]" >&2; exit 2; }
[ -x "$bin" ] || { echo "rt-base-build.sh: no robigo-rt binary at $bin (cargo build --release -p robigo-rtapi)" >&2; exit 1; }
scripts/rt-builder.sh "$bin" > /dev/null
src=$(scripts/r-src.sh)
home=$(scripts/rt-home.sh)
version=4.6.2
# configure's values on the oracle image
which_path=/usr/bin/which
system_abi=linux,gcc,gxx,gfortran,gfortran

rm -rf "$out"
B=$out/build
mkdir -p "$B/bin" "$B/library" "$out/inst/bin" "$out/inst/lib" "$out/logs"
cp "$bin" "$out/inst/bin/robigo-rt"
ln -s "$B" "$out/inst/lib/robigo"
echo "$(sha256sum "$bin" | cut -c1-16) $(basename "$src")" > "$out/stamp"
scripts/rt-builder.sh "$out/inst/bin/robigo-rt" "$(realpath "$bin")" > "$out/builder"

# R: Makeconf.in, share/make/vars.mk (the variables read)
cat > "$B/Makeconf" << EOF
VERSION = $version
SHELL = /bin/bash
ECHO = echo
INSTALL = /usr/bin/install -c
INSTALL_DATA = \${INSTALL} -m 644
MKINSTALLDIRS = /bin/bash \$(top_srcdir)/src/scripts/mkinstalldirs.in
R_OSTYPE = unix
R_PKGS_BASE = base tools compiler
R_PKGS_BASE1 =
R_KEEP_PKG_SOURCE = no
PKG_BUILT_STAMP =
abs_top_srcdir = $src
EOF
touch -d '2000-01-01' "$B/config.status"
# config.status's substitutions for a Makefile.in or DESCRIPTION.in of
# src/library/DIR (DIR empty for src/library itself)
subst() {
  local in=$2 to=$3 sd=$src/src/library${1:+/$1}
  sed -e "s|@srcdir@|$sd|g" -e "s|@top_srcdir@|$src|g" -e "s|@VERSION@|$version|g" \
    -e "s|@WHICH@|$which_path|g" -e "s|@R_SYSTEM_ABI@|$system_abi|g" \
    -e "s|@MAINTAINER_MODE_TRUE@|#|g" -e "s|@BYTE_COMPILE_PACKAGES_FALSE@|#|g" \
    -e "s|@BYTE_COMPILE_PACKAGES_TRUE@||g" -e "s|@NOTANGLE@|notangle|g" "$in" > "$to"
}
mkdir -p "$B/src/library"
subst "" "$src/src/library/Makefile.in" "$B/src/library/Makefile"
for p in profile translations base tools compiler; do
  mkdir -p "$B/src/library/$p"
  subst "$p" "$src/src/library/$p/Makefile.in" "$B/src/library/$p/Makefile"
  if [ -f "$src/src/library/$p/DESCRIPTION.in" ]; then
    subst "$p" "$src/src/library/$p/DESCRIPTION.in" "$B/src/library/$p/DESCRIPTION"
  fi
done
mkdir -p "$B/library/tools/libs"
cp "$home/library/tools/libs/tools.so" "$B/library/tools/libs/"
# R's share/ for the build (licenses, encodings, make/*.mk: rt-home.sh's)
cp -a "$home/share" "$B/share"

cat > "$B/bin/R" << EOF
#!/bin/bash
# the build's R: robigo-rt, its R_HOME the build tree (install layout),
# booted GNU R's way (R_ReplFile of the tree's library/base/R/base: the
# source until base-mklazycomp installs baseloader.R), not M1's loader
unset R_HOME R_SHARE_DIR R_INCLUDE_DIR R_DOC_DIR
export ROBIGO_RT_BOOT=stage0
if [ -n "$shim" ]; then
  { cat "$shim"; cat; } | exec "$out/inst/bin/robigo-rt" "\$@"
else
  exec "$out/inst/bin/robigo-rt" "\$@"
fi
EOF
chmod +x "$B/bin/R"

: > "$out/report.txt"
ran=0
passed=0
# step NAME DIR MAKE-ARGS...: make in src/library/DIR; or a command line
step() {
  local name=$1 dir=$2
  shift 2
  ran=$((ran + 1))
  local log=$out/logs/$name.log status=0
  (cd "$B/src/library/$dir" && "$@") > "$log" 2>&1 || status=$?
  if [ "$status" = 0 ]; then
    passed=$((passed + 1))
    printf '%-16s ok\n' "$name" >> "$out/report.txt"
  else
    # the first error robigo-rt printed (the R error and its next line), or
    # make's last line
    local first
    first=$(grep -m1 -A1 -E '^Error|^robigo-rt:|Execution halted' "$log" | tr '\n' ' ' | cut -c1-300 || true)
    [ -n "$first" ] || first=$(tail -1 "$log" | cut -c1-300)
    printf '%-16s FAILED (%s): %s\n' "$name" "$status" "$first" >> "$out/report.txt"
  fi
}
# R: src/library/Makefile.in's R target
for p in profile translations base tools compiler; do
  step "$p-all" "$p" make all
done
for p in base tools; do
  step "$p-mklazycomp" "$p" make mklazycomp
done
step base-mkdesc base make mkdesc
# R: Makefile.in's R target, its two commands (R_EXE = bin/R --vanilla --no-echo);
# the paths as arguments, so that spaces in OUT do not split them
# shellcheck disable=SC2016 # the inner sh expands $1 and $2
vinstall='echo "tools:::.vinstall_package_${3}_as_RDS(\"$2\", \"base tools compiler\")" | R_DEFAULT_PACKAGES=tools LC_ALL=C "$1" --vanilla --no-echo > /dev/null'
step vdesc . sh -c "$vinstall" sh "$B/bin/R" "$B/library" descriptions
rm -f "$B/library/tools/Meta/nsInfo.rds"
step vns . sh -c "$vinstall" sh "$B/bin/R" "$B/library" namespaces
echo "$passed of $ran steps ran to the end" >> "$out/report.txt"
cat "$out/report.txt"
