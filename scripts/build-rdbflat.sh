#!/bin/sh
# Build the flat lazy-load database cache (crates/robigo-runtime/src/
# rdbflat.rs): enumerate every object and reference key in the installed .rdx
# indices, then pre-inflate every entry into OUTDIR
# (scripts/build-rdbflat.py). Serve it at runtime with
# ROBIGO_RDBFLAT=OUTDIR; a run then reads its base-package objects from
# the cache instead of inflating each entry's zlib stream.
#
# Uncached or changed databases use the runtime's original fetch path.
# ROBIGO_RDBFLAT_LIBRARY optionally selects a different library tree.
#
# Usage: scripts/build-rdbflat.sh [OUTDIR]   (default .robigo/rdbflat)
#   .robigo/bin/robigo must be built; R_HOME (and ROBIGO_NATIVES, if
#   used) as for any robigo run.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$root/.robigo/rdbflat}
log=$(mktemp)
trap 'rm -f "$log"' EXIT

# Never consume an existing flat cache while discovering its replacement.
# An index read failure aborts the build; it must not publish a partial log.
R_DEFAULT_PACKAGES=NULL ROBIGO_RDBFLAT= ROBIGO_RDBLOG= \
    "$root/.robigo/bin/robigo" --vanilla "$root/scripts/build-rdbflat-index.R" >"$log"
python3 "$root/scripts/build-rdbflat.py" "$log" "$out"
