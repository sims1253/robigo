#!/bin/sh
# Build the flat lazy-load database cache (crates/robigo-runtime/src/
# rdbflat.rs): record robigo's lazyLoadDBfetch calls over a battery of
# startup scenarios, then pre-inflate every fetched entry into OUTDIR
# (scripts/build-rdbflat.py). Serve it at runtime with
# ROBIGO_RDBFLAT=OUTDIR; a run then reads its base-package objects from
# the cache instead of inflating each entry's zlib stream.
#
# Entries the battery did not record (a script loading packages the
# battery skipped, say) fall back to the per-fetch path, so the cache is
# always correct — it just stops saving instructions there.
#
# Usage: scripts/build-rdbflat.sh [OUTDIR]   (default .robigo/rdbflat)
#   .robigo/bin/robigo must be built; R_HOME (and ROBIGO_NATIVES, if
#   used) as for any robigo run.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$root/.robigo/rdbflat}
log=$(mktemp)
trap 'rm -f "$log"' EXIT

run() {
    ROBIGO_RDBLOG="$log" "$root/.robigo/bin/robigo" --vanilla "$@" >/dev/null 2>&1 || true
}

# the battery: default startup (all base namespaces plus their lazy
# reference environments), the per-package namespaces again for scripts
# that skip some, and the two non-default package sets
run -e '1'
run -e 'library(methods); library(stats); library(utils); library(grDevices); library(graphics); library(datasets); stopifnot(is.function(lm), is.function(read.table))'
run -e 'e <- new.env(); attach(e); data(mtcars); detach(e); nrow(mtcars)'
R_DEFAULT_PACKAGES=NULL run -e '1'
R_DEFAULT_PACKAGES=utils,stats run -e 'library(stats); lm(dist ~ speed, cars); coef(lm(dist ~ speed, cars))'

python3 "$root/scripts/build-rdbflat.py" "$log" "$out"
