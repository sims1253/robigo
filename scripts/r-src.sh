#!/usr/bin/env bash
# The pinned GNU R 4.6.2 sources: G3's one source input (docs/g3.md L4, §10).
#   dir=$(scripts/r-src.sh)    # the extracted tree, e.g. for robigo-rt's base loader
#   scripts/r-src.sh --tarball # the verified tarball's path
# Downloads the tarball from CRAN into the cache once, checks its sha256 on
# every call, and extracts it once into a directory named after that hash,
# so a tree in the cache is always the pinned one. Never edit the tree.
# The cache defaults to ~/.cache/robigo-research (AGENTS.md); ROBIGO_RESEARCH_CACHE
# overrides it.
set -euo pipefail
version=4.6.2
# CRAN's R-4.6.2.tar.gz, checked against cran.r-project.org on 2026-11-13
sha256=c297040f6392b1dac6e5c0eb398422cfdaedf540ae2205edb9ad0d10cb10ee00
url=https://cran.r-project.org/src/base/R-4/R-$version.tar.gz

cache=${ROBIGO_RESEARCH_CACHE:-$HOME/.cache/robigo-research}/rsrc
tgz=$cache/R-$version.tar.gz
dir=$cache/R-$version-${sha256:0:12}

[ $# -le 1 ] || { echo "r-src: usage: r-src.sh [--tarball]" >&2; exit 2; }
case ${1:-} in
  "" | --tarball) ;;
  *) echo "r-src: unknown argument: $1" >&2; exit 2 ;;
esac

mkdir -p "$cache"
# a failed or interrupted download or extraction leaves nothing behind
tmp=
part=$tgz.part.$$
trap 'rm -rf "$part" ${tmp:+"$tmp"}' EXIT
if [ ! -f "$tgz" ]; then
  echo "r-src: downloading $url" >&2
  curl -fsSL --retry 3 -o "$part" "$url"
  # publish only verified bytes: a bad download must never land in the cache
  echo "$sha256  $part" | sha256sum --check --status ||
    { echo "r-src: $url did not match the pinned sha256 $sha256" >&2; exit 1; }
  mv "$part" "$tgz"
fi
if ! echo "$sha256  $tgz" | sha256sum --check --status; then
  echo "r-src: $tgz does not have the pinned sha256 $sha256; delete it to download again" >&2
  exit 1
fi
if [ "${1:-}" = --tarball ]; then
  echo "$tgz"
  exit 0
fi
# a tree counts only with the stamp written into it before it was published
stamp=.rsrc-pinned
if [ -d "$dir" ] && [ "$(cat "$dir/$stamp" 2> /dev/null)" != "$sha256" ]; then
  echo "r-src: $dir has no stamp for the pinned sha256; delete it to extract again" >&2
  exit 1
fi
if [ ! -d "$dir" ]; then
  tmp=$(mktemp -d "$cache/.R-$version.XXXXXX")
  tar -xzf "$tgz" -C "$tmp"
  echo "$sha256" > "$tmp/R-$version/$stamp"
  # GNU mv: -T renames onto $dir instead of nesting into it (coreutils required).
  # Another process may have finished first: keep whichever tree landed.
  if ! mv -T "$tmp/R-$version" "$dir"; then
    [ "$(cat "$dir/$stamp" 2> /dev/null)" = "$sha256" ] || { echo "r-src: could not create $dir" >&2; exit 1; }
  fi
fi
echo "$dir"
