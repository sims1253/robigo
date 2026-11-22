#!/usr/bin/env bash
# Refresh vendor/libdeflate/ from libdeflate's release tarball:
#   scripts/vendor-libdeflate.sh PATH/TO/libdeflate-1.26.tar.gz
# The tarball is upstream's release asset
# (https://github.com/ebiggers/libdeflate/releases/download/v1.26/libdeflate-1.26.tar.gz);
# its sha256 is checked before anything is copied. Only the zlib
# decompressor's sources are taken (vendor/libdeflate/README.md lists them
# and why), unmodified. To update: change VERSION and SHA256 to the new
# release's, run this, rebuild, and re-measure (README.md, "Updating").
set -euo pipefail
VERSION=1.26
SHA256=125856d4656e0feab660f94842f835923410c9281fedbcee64598c918da42b5a
FILES=(
  COPYING
  libdeflate.h
  common_defs.h
  lib/lib_common.h
  lib/deflate_constants.h
  lib/zlib_constants.h
  lib/decompress_template.h
  lib/cpu_features_common.h
  lib/deflate_decompress.c
  lib/zlib_decompress.c
  lib/adler32.c
  lib/utils.c
  lib/x86/cpu_features.c
  lib/x86/cpu_features.h
  lib/x86/decompress_impl.h
  lib/x86/adler32_impl.h
  lib/x86/adler32_template.h
  lib/arm/cpu_features.c
  lib/arm/cpu_features.h
  lib/arm/adler32_impl.h
)
tarball="$1"
echo "$SHA256  $tarball" | sha256sum --check --quiet
dst="$(cd "$(dirname "$0")/.." && pwd)/vendor/libdeflate"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
tar -xzf "$tarball" -C "$tmp"
src="$tmp/libdeflate-$VERSION"
# (the README is ours: keep it across a refresh)
find "$dst" -mindepth 1 ! -name README.md -delete 2>/dev/null || true
for f in "${FILES[@]}"; do
  mkdir -p "$dst/$(dirname "$f")"
  cp "$src/$f" "$dst/$f"
done
echo "vendored ${#FILES[@]} files of libdeflate $VERSION into $dst"
