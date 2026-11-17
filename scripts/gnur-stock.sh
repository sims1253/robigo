#!/usr/bin/env bash
# The instruction bars' GNU R rival (the lead's F2, 2026-11-17;
# docs/ledger/g3/2026-11-17-a0b-stock-rival.md): GNU R 4.6.2 from the pinned
# tarball (scripts/r-src.sh), built with GNU R's literal default configure in
# this machine's toolchain, so static libR, no hardening and the compiler's
# default `-g -O2`.
#   scripts/gnur-stock.sh PREFIX   # build and install into PREFIX
# Prints the installed R_HOME, the directory perf.py rt-gnur takes as
# R_HOME_DIR (its exec/R is RSCRIPT_BIN). PREFIX/robigo-stock.txt records the
# configure line and the flags Makeconf got. The only argument configure gets
# is --prefix; flags from the environment are cleared, so the build is the
# default whatever the caller's shell sets.
set -euo pipefail
[ $# -eq 1 ] || { echo "gnur-stock: usage: gnur-stock.sh PREFIX" >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
prefix=$(mkdir -p "$1" && cd "$1" && pwd)
tgz=$("$root/scripts/r-src.sh" --tarball)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -xzf "$tgz" -C "$work"
cd "$work"/R-*
unset CFLAGS CXXFLAGS FFLAGS FCFLAGS CPPFLAGS LDFLAGS CC CXX FC F77 R_HOME R_LIBS R_LIBS_USER R_LIBS_SITE LIBnn
./configure --prefix="$prefix" > "$work/configure.log" 2>&1 ||
  { tail -40 "$work/configure.log" >&2; exit 1; }
make -j"$(nproc)" > "$work/make.log" 2>&1 || { tail -40 "$work/make.log" >&2; exit 1; }
make install > "$work/install.log" 2>&1 || { tail -40 "$work/install.log" >&2; exit 1; }
r_home=$("$prefix/bin/R" RHOME)
{
  echo "configure: ./configure --prefix=$prefix"
  grep -E '^(CC|CFLAGS|LDFLAGS) =' Makeconf
  if [ -e "$r_home/lib/libR.so" ]; then echo "libR: shared"; else echo "libR: static"; fi
} > "$prefix/robigo-stock.txt"
echo "$r_home"
