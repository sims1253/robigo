#!/usr/bin/env bash
# What a robigo-rt binary is, for the trees it builds to record
# (scripts/rt-home-db.sh, scripts/rt-base-build.sh) and
# scripts/rt-rdb-compare.sh to report: its path, sha256, GNU build ID and
# the commit it was built from (the marker build.rs embeds:
# `robigo-rt-commit:<sha>`, the commit checked out at the build; an edit
# not committed shows only in the sha256). Fails when the binary carries
# no commit (built before the marker, or not a robigo-rt).
#   scripts/rt-builder.sh BIN > FILE
set -euo pipefail
[ $# -eq 1 ] && [ -n "$1" ] || { echo "usage: scripts/rt-builder.sh BIN" >&2; exit 2; }
bin=$(realpath "$1")
[ -x "$bin" ] || { echo "rt-builder.sh: no executable at $1" >&2; exit 2; }
commit=$(grep -a -o -m1 'robigo-rt-commit:[0-9a-f]\{40\}' "$bin" | head -1 | cut -d: -f2) || true
[ -n "$commit" ] || { echo "rt-builder.sh: $bin carries no robigo-rt-commit marker: not a robigo-rt this checkout can identify" >&2; exit 1; }
build_id=$(readelf -n "$bin" 2>/dev/null | sed -n 's/^ *Build ID: *//p' | head -1)
echo "path: $bin"
echo "sha256: $(sha256sum "$bin" | cut -d' ' -f1)"
echo "build-id: ${build_id:-none}"
echo "commit: $commit"
