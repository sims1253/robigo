#!/bin/sh
# The capi corpus's build step (corpora.toml key `build`): compile every
# C case source with GNU R's toolchain. The .so files land next to the
# sources in support/ (gitignored) and are linked into each case's working
# directory by scripts/corpus.py. R CMD SHLIB links against libR, so the
# undefined symbols of a built .so are exactly the libR API its case
# exercises; scripts/capi-census.py coverage measures them.
set -e
cd "$(dirname "$0")"
for c in support/*.c; do
  echo "R CMD SHLIB $c"
  R CMD SHLIB "$c" > /dev/null
done
