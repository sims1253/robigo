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

mkdir -p "$cache"
if [ ! -f "$tgz" ]; then
  echo "r-src: downloading $url" >&2
  curl -fsSL --retry 3 -o "$tgz.part.$$" "$url"
  mv "$tgz.part.$$" "$tgz"
fi
if ! echo "$sha256  $tgz" | sha256sum --check --status; then
  echo "r-src: $tgz does not have the pinned sha256 $sha256; delete it to download again" >&2
  exit 1
fi
if [ "${1:-}" = --tarball ]; then
  echo "$tgz"
  exit 0
fi
if [ ! -d "$dir" ]; then
  tmp=$(mktemp -d "$cache/.R-$version.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  tar -xzf "$tgz" -C "$tmp"
  # another process may have finished first: keep whichever tree landed
  mv -T "$tmp/R-$version" "$dir" 2>/dev/null || [ -d "$dir" ]
fi
echo "$dir"
