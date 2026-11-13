#!/bin/sh
# Install pinned sources using GNU R $ROBIGO_R_VERSION (default 4.5.0, set
# by scripts/corpus.py: the recording's version, or 4.5.0 for robigo),
# without bytecode. Publish the library only after every install succeeds.
set -eu
here=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
lib="$here/support/lib"
want=${ROBIGO_R_VERSION:-4.5.0}
# the version is part of the stamp: a library built by another R is rebuilt
stamp=$( (cat "$here/packages.txt" "$here/build.sh"; echo "$want") | sha256sum | cut -d ' ' -f 1)
if [ -f "$lib/.corpus-packages" ] && [ "$(cat "$lib/.corpus-packages")" = "$stamp" ]; then
    exit 0
fi
R --version | head -1 | grep -qF "R version $want " || {
    echo "cran-purer: building requires GNU R $want" >&2
    exit 1
}
mkdir -p "$here/support"  # holds only the built library, so a fresh checkout lacks it
staging=$(mktemp -d "$here/support/install.XXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM
chmod 755 "$staging"
while read -r name version digest licence; do
    case "$name" in ''|'#'*) continue ;; esac
    archive="$here/src/${name}_${version}.tar.gz"
    printf '%s  %s\n' "$digest" "$archive" | sha256sum -c -
    R CMD INSTALL --no-docs --no-html --no-byte-compile --library="$staging" "$archive" >/dev/null
done < "$here/packages.txt"
printf '%s\n' "$stamp" > "$staging/.corpus-packages"
rm -rf "$lib"
mv "$staging" "$lib"
