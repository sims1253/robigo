#!/bin/sh
# Install pinned sources using GNU R 4.5.0, without bytecode, for both
# the oracle and robigo. Publish the library only after every install succeeds.
set -eu
here=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
lib="$here/support/lib"
stamp=$(cat "$here/packages.txt" "$here/build.sh" | sha256sum | cut -d ' ' -f 1)
if [ -f "$lib/.corpus-packages" ] && [ "$(cat "$lib/.corpus-packages")" = "$stamp" ]; then
    exit 0
fi
R --version | head -1 | grep -q '4\.5\.0' || {
    echo "cran-purer: building requires GNU R 4.5.0" >&2
    exit 1
}
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
