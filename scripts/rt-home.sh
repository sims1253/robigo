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
#   library/tools/libs/tools.so
#                             tools' native code (src/library/tools/src, its
#                             Makefile.in's flags: -DHAVE_CONFIG_H and
#                             C_VISIBILITY), built against robigo's headers for
#                             GNU R's natives (crates/robigo-rtapi/internal,
#                             docs/g3.md §10) and the C API's, then the
#                             tarball's public ones
# The tree is rebuilt when the tarball's tree or this script changes (the
# stamp holds both hashes).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-.robigo/rt-home}
src=$(scripts/r-src.sh)
version=4.6.2
# configure's WHICH on the oracle image (Sys.which's body there)
which_path=/usr/bin/which
# robigo_layout.h, which robigo-heap's build writes (its out_dir: the one
# build script run of robigo-heap's; cargo's errors stay on stderr)
layout=$(cargo build -q --release --locked -p robigo-heap --message-format=json | python3 -c '
import json, sys
dirs = []
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "build-script-executed" and "/robigo-heap#" in m.get("package_id", ""):
        dirs.append(m["out_dir"] + "/include")
if len(dirs) != 1:
    sys.exit(f"rt-home.sh: {len(dirs)} build script runs of robigo-heap, not one")
print(dirs[0])
')
[ -f "$layout/robigo_layout.h" ] || { echo "rt-home.sh: no robigo_layout.h from robigo-heap's build" >&2; exit 1; }
cc=${CC:-cc}
headers=(crates/robigo-rtapi/internal crates/robigo-rtapi/include crates/robigo-rtcore/include "$layout")
# the natives' inputs: the headers, in C order whatever the caller's locale
# (the stamp is their hash), and the compiler (the sources are the
# tarball's, named by its directory)
hfiles=()
for d in "${headers[@]}"; do
  mapfile -t hs < <(find "$d" -maxdepth 1 -name '*.h' -type f | LC_ALL=C sort)
  [ "${#hs[@]}" -gt 0 ] || { echo "rt-home.sh: no headers in $d" >&2; exit 1; }
  hfiles+=("${hs[@]}")
done
natives=$( (cat "${hfiles[@]}" && "$cc" --version) | sha256sum | cut -c1-16)
stamp="$(basename "$src") $(sha256sum "$0" | cut -c1-16) $natives"
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
mkdir -p "$tmp/build/tools" "$tmp/library/tools/libs"
for f in "$lib/tools/src"/*.c; do
  "$cc" -std=gnu11 -O2 -fPIC -fvisibility=hidden -DHAVE_CONFIG_H "${headers[@]/#/-I}" -I"$src/src/include" \
    -I"$lib/tools/src" -c "$f" -o "$tmp/build/tools/$(basename "$f" .c).o"
done
"$cc" -shared -o "$tmp/library/tools/libs/tools.so" "$tmp/build/tools"/*.o
rm -rf "$tmp/build"
echo "$stamp" > "$tmp/.rt-home"
rm -rf "$out"
mv "$tmp" "$out"
trap - EXIT
echo "$out"
