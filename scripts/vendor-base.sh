#!/usr/bin/env bash
# Refresh rlib/ from an R source tree: scripts/vendor-base.sh PATH/TO/R-x.y.z
set -euo pipefail
src="$1/src/library"
dst="$(dirname "$0")/../rlib"
rm -rf "$dst/base/R"
mkdir -p "$dst/base/R/unix" "$dst/profile"
cp "$src"/base/R/*.R "$dst/base/R/"
cp "$src"/base/R/unix/*.R "$dst/base/R/unix/"
cp "$src"/profile/Common.R "$src"/profile/Rprofile.unix "$dst/profile/"
cp "$1/COPYING" "$dst/COPYING"
echo "vendored $(ls "$dst"/base/R/*.R "$dst"/base/R/unix/*.R | wc -l) base files from $1"
