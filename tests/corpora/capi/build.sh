#!/bin/sh
# The capi corpus's build step (corpora.toml key `build`): compile every
# C case source. The .so files land next to the sources in support/
# (gitignored) and are linked into each case's working directory by
# scripts/corpus.py. With a robigo binary ($ROBIGO, else
# .robigo/bin/robigo) the cases are built by `robigo CMD SHLIB` against
# robigo's headers and R_HOME (docs/core.md §8; the home is made by
# scripts/robigo-home.sh when missing), otherwise by GNU R's
# `R CMD SHLIB`. Either links against libR, so the undefined symbols of a
# built .so are exactly the libR API its case exercises;
# scripts/capi-census.py coverage measures them.
set -e
cd "$(dirname "$0")"
root=$(cd ../../.. && pwd)
robigo=${ROBIGO:-$root/.robigo/bin/robigo}
if [ -x "$robigo" ]; then
  robigo=$(cd "$(dirname "$robigo")" && pwd)/$(basename "$robigo")
  home=$root/.robigo/home
  if [ ! -f "$home/etc/Makeconf" ] || [ "$(readlink "$home/bin/exec/R")" != "$robigo" ]; then
    sh "$root/scripts/robigo-home.sh" "${R_HOME:-$(R RHOME)}" "$robigo" "$home"
  fi
  shlib="$home/bin/R CMD SHLIB"
else
  shlib="R CMD SHLIB"
fi
# rebuild everything when the toolchain changes
stamp=support/.built-with
if [ "$(cat "$stamp" 2>/dev/null)" != "$shlib" ]; then
  rm -f support/*.o support/*.so
  echo "$shlib" > "$stamp"
fi
for c in support/*.c; do
  echo "$shlib $c"
  $shlib "$c" > /dev/null
done
