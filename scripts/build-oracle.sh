#!/usr/bin/env bash
# Build GNU R 4.5.0, the oracle, for machines whose system R is another
# version: conformance and scripts/regen-expected.sh need the real one.
#   scripts/build-oracle.sh
#   export PATH=~/.cache/robigo-research/R-4.5.0/bin:$PATH
#   export R_HOME=$(R RHOME)
# Sources come from ~/.cache/robigo-research/rsrc (AGENTS.md). glibc is the
# system's, so results can differ from CI's Debian in the last bit.
set -euo pipefail
cache=~/.cache/robigo-research
src=$cache/rsrc/R-4.5.0
if [ ! -d "$src" ]; then
  mkdir -p "$cache/rsrc"
  curl -fsSL https://cran.r-project.org/src/base/R-4/R-4.5.0.tar.gz | tar -xz -C "$cache/rsrc"
fi
build=$cache/R-4.5.0-build
mkdir -p "$build"
cd "$build"
"$src/configure" --prefix="$cache/R-4.5.0" --without-recommended-packages --with-x=no --disable-java \
  --without-tcltk --with-readline=no --without-cairo --without-libtiff --without-jpeglib --without-libpng
make -j"$(nproc)"
make install
"$cache/R-4.5.0/bin/Rscript" --version
