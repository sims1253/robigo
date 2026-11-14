#!/usr/bin/env bash
# Build robigo-rt's package C (tests/rt/capi/support/*.c) into OUT, one
# NAME.so per NAME.c, for the runtime that will load it (docs/g3.md §11):
#   scripts/rt-capi-build.sh OUT          # robigo-rt: GNU R 4.6.2's public
#                                         # headers with robigo's Rinternals.h
#                                         # in front (crates/robigo-rtapi/include)
#   scripts/rt-capi-build.sh --gnur OUT   # the Rscript on PATH: its own headers
# The cases load them from RT_CAPI_LIBS, which the runners set to OUT:
# robigo-rt's conformance test (crates/robigo-rtapi/tests/conformance.rs)
# and scripts/regen-expected.sh when it records tests/rt. No R CMD SHLIB:
# cc -fPIC -shared, undefined symbols resolved by the runtime that loads
# the object. CC overrides the compiler; a library is rebuilt only when its
# source or the headers' directory changed.
set -euo pipefail
cd "$(dirname "$0")/.."
gnur=0
if [ "${1:-}" = --gnur ]; then
  gnur=1
  shift
fi
[ $# -eq 1 ] || { echo "usage: scripts/rt-capi-build.sh [--gnur] OUT" >&2; exit 2; }
out=$1
mkdir -p "$out"
if [ "$gnur" = 1 ]; then
  # (the installation's own include path, wherever it put R.h)
  read -r -a inc <<< "$(R CMD config --cppflags)"
else
  inc=(-Icrates/robigo-rtapi/include -Icrates/robigo-rtcore/include -I"$(scripts/r-src.sh)/src/include")
fi
cc=${CC:-cc}
for c in tests/rt/capi/support/*.c; do
  so=$out/$(basename "${c%.c}").so
  if [ "$so" -nt "$c" ] && [ "$so" -nt crates/robigo-rtapi/include/Rinternals.h ] &&
     [ "$so" -nt crates/robigo-rtcore/include/robigo_capi.h ]; then
    continue
  fi
  "$cc" -fPIC -shared -O2 -g "${inc[@]}" -o "$so.tmp" "$c"
  mv "$so.tmp" "$so"
done
