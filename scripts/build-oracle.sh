#!/usr/bin/env bash
# Build a GNU R oracle (docs/verification.md, "Oracles") for machines whose
# system R is another version: conformance, scripts/regen-expected.sh and
# scripts/corpus.py record need the real one.
#
#   scripts/build-oracle.sh [VERSION]        # from source (default 4.5.0)
#   export PATH=~/.cache/robigo-research/R-4.5.0/bin:$PATH
#   export R_HOME=$(R RHOME)
#
#   scripts/build-oracle.sh --image VERSION  # the oracle image, and wrappers
#   export PATH=~/.cache/robigo-oracle/4.6.2/bin:$PATH ROBIGO_R_VERSION=4.6.2
#
# The source is the tarball scripts/r-tarballs.sha256 pins, checked by hash
# (unpacked once into ~/.cache/robigo-research/rsrc, AGENTS.md). A source
# build uses the system's glibc, so results can differ from CI's Debian in
# the last bit; record with an image. `--image` builds robigo-oracle:VERSION
# (.github/ci-image/oracle.Dockerfile, the CI image's oracle-VERSION target)
# and writes ~/.cache/robigo-oracle/VERSION/bin/{R,Rscript}, which run it
# with $HOME mounted and without docker's seccomp filter (as
# scripts/oracle-run.sh, which says why). 4.5.0's image is Debian's
# r-base-core instead (tests/packages/Dockerfile, scripts/package-suite.py).
# ROBIGO_RESEARCH moves the source-build cache.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
image=false
if [ "${1:-}" = --image ]; then
  image=true
  shift
fi
version=${1:-4.5.0}
pin=$(grep -F "  R-$version.tar.gz" "$root/scripts/r-tarballs.sha256") || {
  echo "error: R $version is not an oracle version (scripts/r-tarballs.sha256)" >&2
  exit 1
}

if $image; then
  if [ "$version" = 4.5.0 ]; then
    echo "error: 4.5.0's image is Debian's: docker build -t robigo-oracle:4.5.0 tests/packages" >&2
    exit 1
  fi
  "$root/scripts/ci-image.sh" build "oracle-$version"
  ref=$("$root/scripts/ci-image.sh" refs | sed -n "s/^oracle_${version//./_}=//p")
  docker tag "$ref" "robigo-oracle:$version"
  bin=~/.cache/robigo-oracle/$version/bin
  mkdir -p "$bin"
  for exe in R Rscript; do
    cat > "$bin/$exe" <<EOF
#!/bin/sh
# GNU R $version in robigo-oracle:$version (scripts/build-oracle.sh --image)
exec docker run --rm -i --security-opt seccomp=unconfined -u \$(id -u):\$(id -g) -v "\$HOME:\$HOME" -w "\$PWD" -e HOME="\$HOME" -e LANGUAGE -e LC_ALL -e TZ -e R_DEFAULT_PACKAGES -e TZDIR robigo-oracle:$version $exe "\$@"
EOF
    chmod +x "$bin/$exe"
  done
  "$bin/Rscript" --version
  exit 0
fi

cache=${ROBIGO_RESEARCH:-~/.cache/robigo-research}
src=$cache/rsrc/R-$version
if [ ! -d "$src" ]; then
  mkdir -p "$cache/rsrc"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  curl -fsSL -o "$tmp/R-$version.tar.gz" "https://cran.r-project.org/src/base/R-${version%%.*}/R-$version.tar.gz"
  (cd "$tmp" && printf '%s\n' "$pin" | sha256sum -c -)
  tar -xzf "$tmp/R-$version.tar.gz" -C "$cache/rsrc"
fi
build=$cache/R-$version-build
mkdir -p "$build"
cd "$build"
"$src/configure" --prefix="$cache/R-$version" --without-recommended-packages --with-x=no --disable-java \
  --without-tcltk --with-readline=no --without-cairo --without-libtiff --without-jpeglib --without-libpng
make -j"$(nproc)"
make install
"$cache/R-$version/bin/Rscript" --version
