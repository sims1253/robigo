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
#   share/                    share/Makefile.in's install-sources: R/*.R,
#                             bibliographies/*.R, dictionaries/*.txt,
#                             encodings/*, java/*, licenses/*, make/*.mk,
#                             Rd/macros/*, sh/*.sh, texmf/bibtex/{bib,bst}/*,
#                             texmf/tex/latex/* (no zoneinfo: the oracle's
#                             configure uses the system's; install-sources2's
#                             .rds files are made by R, L4's build)
#   etc/Renviron              etc/Makefile.in's: Renviron.in with configure's
#                             values on the oracle image (and its Debian line)
#   include/Rmath.h           src/include/Makefile's Rmath.h: config.status's
#                             Rmath.h0 from Rmath.h0.in (@PACKAGE_VERSION@, and
#                             @RMATH_HAVE_WORKING_LOG1P@ as configure finds it
#                             on the oracle image: glibc's log1p works)
#   library/tools/libs/tools.so
#                             tools' native code (src/library/tools/src, its
#                             Makefile.in's flags: -DHAVE_CONFIG_H and
#                             C_VISIBILITY), built against robigo's headers for
#                             GNU R's natives (crates/robigo-rtapi/internal,
#                             docs/g3.md §10) and the C API's, then the
#                             tarball's public ones and the tree's include/:
#                             nothing from an installed R
# The tree is rebuilt when the tarball's tree or this script changes (the
# stamp holds both hashes).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-.robigo/rt-home}
# (no trailing slash or /.: the lock is the tree's sibling)
while [ "$out" != / ] && { [ "${out%/}" != "$out" ] || [ "${out%/.}" != "$out" ]; }; do
  out=${out%/}
  out=${out%/.}
done
[ -n "$out" ] || out=/
# one build of the tree at a time (another's rm and mv would interleave
# with this one's, or swap the tree under scripts/rt-home-db.sh's copy)
mkdir -p "$(dirname "$out")"
exec 8> "$out.lock"
flock 8
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
# R: share/Makefile.in install-sources: each glob in its order, expanded once
# against the tarball's share/ (installdirs makes each one's directory)
share_globs=('R/*.R' 'bibliographies/*.R' 'dictionaries/*.txt' 'encodings/*' 'java/*' 'licenses/*'
  'make/*.mk' 'Rd/macros/*' 'sh/*.sh' 'texmf/bibtex/bib/*' 'texmf/bibtex/bst/*' 'texmf/tex/latex/*')
for g in "${share_globs[@]}"; do
  mkdir -p "$tmp/share/${g%/*}"
  # shellcheck disable=SC2206 # the glob, as the Makefile's `for f in $(srcdir)/$g`
  files=("$src"/share/$g)
  for f in "${files[@]}"; do
    # (an unmatched glob stays itself, which is no file)
    [ -f "$f" ] || { echo "rt-home.sh: share/$g: '${f#"$src"/}' is not a regular file of the tarball" >&2; exit 1; }
  done
  cp -p "${files[@]}" "$tmp/share/${g%/*}/"
done
# R: etc/Makefile.in's Renviron, config.status's etc/Renviron from
# Renviron.in, with configure's values on the oracle image; then the line
# Debian's line (set_R_LIBS_USER_and_SITE_in_Renviron.patch), written and
# then checked as this one string
libs_site="R_LIBS_SITE=\${R_LIBS_SITE:-'/usr/local/lib/R/site-library:%S'}"
# Debian's set_R_LIBS_USER_and_SITE_in_Renviron.patch changes, as the image
# reproduces it (.github/ci-image/oracle.Dockerfile)
mkdir -p "$tmp/etc"
sed -e 's|@configure_input@|etc/Renviron.  Generated from Renviron.in by configure.|' \
  -e 's|@R_PLATFORM@|x86_64-pc-linux-gnu|' -e 's|@R_PAPERSIZE@|letter|' -e 's|@R_PRINTCMD@|/usr/bin/lpr|' \
  -e 's|@R_RD4PDF@|times,inconsolata,hyper|' -e 's|@TEXI2DVICMD@|texi2dvi|' -e 's|@R_GZIPCMD@|/bin/gzip -n|' \
  -e 's|@R_UNZIPCMD@|/usr/bin/unzip|' -e 's|@R_ZIPCMD@|/usr/bin/zip|' -e 's|@R_BZIPCMD@|/bin/bzip2|' \
  -e 's|@R_BROWSER@|xdg-open|' -e 's|@PAGER@|/usr/bin/pager|' -e 's|@R_PDFVIEWER@|/usr/bin/xdg-open|' \
  -e 's|@LN_S@|ln -s|' -e 's|@MAKE@|make|' -e 's|@SED@|/bin/sed|' -e 's|@TAR@|/bin/tar|' \
  -e 's|@STRIP_SHARED_LIB@|strip --strip-unneeded|' -e 's|@STRIP_STATIC_LIB@|strip --strip-debug|' \
  -e "s|^R_LIBS_SITE=.*|$libs_site|" \
  "$src/etc/Renviron.in" > "$tmp/etc/Renviron"
# every configure token substituted and Debian's line in place, or the
# file is not the oracle's
if grep -qE '@[A-Za-z_]+@' "$tmp/etc/Renviron"; then
  grep -oE '@[A-Za-z_]+@' "$tmp/etc/Renviron" | sed 's/^/rt-home.sh: etc\/Renviron keeps /' >&2
  exit 1
fi
grep -qxF "$libs_site" "$tmp/etc/Renviron" ||
  { echo "rt-home.sh: Renviron.in's R_LIBS_SITE line is not where rt-home.sh expects it" >&2; exit 1; }
mkdir -p "$tmp/include"
sed -e "s/@PACKAGE_VERSION@/$version/" -e "s/@RMATH_HAVE_WORKING_LOG1P@/# define HAVE_WORKING_LOG1P 1/" \
  "$src/src/include/Rmath.h0.in" > "$tmp/include/Rmath.h"
mkdir -p "$tmp/build/tools" "$tmp/library/tools/libs"
for f in "$lib/tools/src"/*.c; do
  "$cc" -std=gnu11 -O2 -fPIC -fvisibility=hidden -DHAVE_CONFIG_H "${headers[@]/#/-I}" -I"$src/src/include" \
    -I"$tmp/include" -I"$lib/tools/src" -c "$f" -o "$tmp/build/tools/$(basename "$f" .c).o"
done
"$cc" -shared -o "$tmp/library/tools/libs/tools.so" "$tmp/build/tools"/*.o
rm -rf "$tmp/build"
echo "$stamp" > "$tmp/.rt-home"
rm -rf "$out"
mv "$tmp" "$out"
trap - EXIT
echo "$out"
