#!/usr/bin/env bash
# L1's bar (m2m3-plan §B L1): GNU R 4.6.2's own lazy-load databases (base,
# compiler and tools, from the oracle image robigo-oracle:4.6.2) unserialize
# record by record on robigo-rt and serialize back to the same bytes; the
# records holding one of base's compact sequences (ALTREP) are counted apart
# until robigo-rt has those classes (AC). Runs crates/robigo-rt/tests/rdb_roundtrip.rs on a copy of the
# image's library, against GNU R's own round trip of each record (written
# beside it as PKG.roundtrip by GNU R in the image): GNU R's reader changes
# a few records (a forced promise's PRENV, R_NilValue, becomes the base
# environment, serialize.c's ReadItem_Iterative), so its re-serialization of
# those is not their bytes either, and robigo-rt's must be GNU R's.
#   scripts/rt-rdb-roundtrip.sh [DIR]              # DIR: where to copy (default .robigo/oracle-lib)
#   scripts/rt-rdb-roundtrip.sh --data-only DIR    # make the data only (CI's rdb-data job)
# Where GNU R 4.6.2 is the R on PATH (inside the oracle image: CI), the data
# is made with it directly; elsewhere in the image robigo-oracle:4.6.2.
set -euo pipefail
cd "$(dirname "$0")/.."
data_only=
if [ "${1:-}" = --data-only ]; then
  data_only=1
  shift
  [ $# -eq 1 ] || { echo "usage: scripts/rt-rdb-roundtrip.sh --data-only DIR" >&2; exit 2; }
fi
dir=${1:-.robigo/oracle-lib}
mkdir -p "$dir"
dir=$(cd "$dir" && pwd)
# the data's maker, run by the oracle's sh with OUT the output directory
# shellcheck disable=SC2016 # single quotes on purpose: the inner sh expands it
make_data='
  for p in base compiler tools; do
    mkdir -p "$OUT/$p/R" && cp "$(R RHOME)/library/$p/R/$p.rdb" "$(R RHOME)/library/$p/R/$p.rdx" "$OUT/$p/R/"
  done
  Rscript --vanilla -e "
    envs <- list()
    inh <- function(n) { if (is.null(envs[[n]])) envs[[n]] <<- new.env(); envs[[n]] }
    outh <- function(e) { for (n in names(envs)) if (identical(envs[[n]], e)) return(n); NULL }
    for (p in c(\"base\", \"compiler\", \"tools\")) {
      d <- file.path(Sys.getenv(\"OUT\"), p, \"R\")
      x <- readRDS(file.path(d, paste0(p, \".rdx\")))
      con <- file(file.path(d, paste0(p, \".rdb\")), \"rb\")
      # written beside, then renamed: an interrupted run leaves no partial
      # .roundtrip for the test to read
      f <- file.path(d, paste0(p, \".roundtrip\"))
      out <- file(paste0(f, \".tmp\"), \"wb\")
      keys <- c(x[[\"variables\"]], x[[\"references\"]])
      for (nm in names(keys)) {
        k <- keys[[nm]]
        seek(con, k[1L]); b <- readBin(con, \"raw\", k[2L])
        v <- unserialize(memDecompress(b[-(1:4)], \"gzip\"), refhook = inh)
        r <- serialize(v, NULL, refhook = outh)
        writeLines(c(nm, as.character(length(r))), out)
        writeBin(r, out)
      }
      close(out); close(con)
      if (!file.rename(paste0(f, \".tmp\"), f)) stop(\"cannot rename \", f, \".tmp\")
    }"'
# captured first: a pipeline into head or grep -q could end R with SIGPIPE,
# which pipefail would report as a failure
rv=$(R --version 2> /dev/null) || rv=
case "${rv%%$'\n'*}" in
  "R version 4.6.2 "*) local_r=1 ;;
  *) local_r= ;;
esac
if [ -n "$local_r" ]; then
  OUT=$dir LC_ALL=C R_DEFAULT_PACKAGES=NULL sh -c "$make_data"
else
  docker image inspect robigo-oracle:4.6.2 > /dev/null 2>&1 || {
    echo "error: no GNU R 4.6.2 on PATH and no image robigo-oracle:4.6.2; run scripts/build-oracle.sh --image 4.6.2" >&2
    exit 1
  }
  docker run --rm -u "$(id -u):$(id -g)" -v "$dir:/out" -e OUT=/out -e LC_ALL=C -e R_DEFAULT_PACKAGES=NULL \
    robigo-oracle:4.6.2 sh -c "$make_data"
fi
[ -n "$data_only" ] && exit 0
ROBIGO_ORACLE_LIBRARY=$dir cargo test --release -p robigo-rt --test rdb_roundtrip -- --nocapture
