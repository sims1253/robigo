#!/usr/bin/env bash
# robigo-rt's R_HOME tree (m2m3-plan F7b, option A): made from the pinned
# GNU R 4.6.2 tarball's tree (scripts/r-src.sh) as GNU R's own build makes
# its library before the databases exist, so that robigo-rt evaluates base
# from source (L2's stage 0, main.c's R_ReplFile of library/base/R/base) and
# loads namespaces from source (loadNamespace's sys.source of
# library/PKG/R/PKG). L4's build adds the databases to the same tree.
#   scripts/rt-home.sh [DIR]    # default .robigo/rt-home
# What each file is, after the build rule that makes it:
#   library/base/R/base       basepkg.mk mkRbase: `cat $(RSRC)` (R/*.R, R/unix/*.R
#                             in LC_COLLATE=C order), @WHICH@ substituted
#   library/base/R/Rprofile   profile/Makefile: Common.R then Rprofile.unix
#   library/PKG/DESCRIPTION   DESCRIPTION.in with @VERSION@ (config.status)
#   library/PKG/NAMESPACE     the package's NAMESPACE
#   library/PKG/R/PKG         mkR1/mkRbase's all.R, for compiler and tools
# The tree is rebuilt when the tarball's tree or this script changes (the
# stamp holds both hashes).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-.robigo/rt-home}
src=$(scripts/r-src.sh)
version=4.6.2
# configure's WHICH on the oracle image (Sys.which's body there)
which_path=/usr/bin/which
stamp="$(basename "$src") $(sha256sum "$0" | cut -c1-16)"
if [ -f "$out/.rt-home" ] && [ "$(cat "$out/.rt-home")" = "$stamp" ]; then
  echo "$out"
  exit 0
fi
tmp=$(mktemp -d "${out%/*}/.rt-home.XXXXXX" 2>/dev/null || { mkdir -p "${out%/*}"; mktemp -d "${out%/*}/.rt-home.XXXXXX"; })
trap 'rm -rf "$tmp"' EXIT
lib=$src/src/library
desc() { sed -e "s/@VERSION@/$version/g" "$lib/$1/DESCRIPTION.in" > "$tmp/library/$1/DESCRIPTION"; }
mkdir -p "$tmp/library/base/R"
# shellcheck disable=SC2046 # the file list, as the Makefile's RSRC
(cd "$lib/base" && LC_COLLATE=C cat $(LC_COLLATE=C ls R/*.R R/unix/*.R)) |
  sed -e "s:@WHICH@:$which_path:" > "$tmp/library/base/R/base"
cat "$lib/profile/Common.R" "$lib/profile/Rprofile.unix" > "$tmp/library/base/R/Rprofile"
desc base
for pkg in compiler tools; do
  mkdir -p "$tmp/library/$pkg/R"
  desc "$pkg"
  cp "$lib/$pkg/NAMESPACE" "$tmp/library/$pkg/NAMESPACE"
  # shellcheck disable=SC2046
  (cd "$lib/$pkg" && LC_COLLATE=C cat $(LC_COLLATE=C ls R/*.R)) > "$tmp/library/$pkg/R/$pkg"
done
echo "$stamp" > "$tmp/.rt-home"
rm -rf "$out"
mv "$tmp" "$out"
trap - EXIT
echo "$out"
