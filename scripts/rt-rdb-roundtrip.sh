#!/usr/bin/env bash
# L1's bar (m2m3-plan §B L1): GNU R 4.6.2's own lazy-load databases (base,
# compiler and tools, from the oracle image robigo-oracle:4.6.2) unserialize
# record by record on robigo-rt and serialize back to the same bytes; the
# records with byte code are counted apart until robigo-rt has BCODESXP
# (C1-BC). Runs crates/robigo-rt/tests/rdb_roundtrip.rs on a copy of the
# image's library, against GNU R's own round trip of each record (written
# beside it as PKG.roundtrip by GNU R in the image): GNU R's reader changes
# a few records (a forced promise's PRENV, R_NilValue, becomes the base
# environment, serialize.c's ReadItem_Iterative), so its re-serialization of
# those is not their bytes either, and robigo-rt's must be GNU R's.
#   scripts/rt-rdb-roundtrip.sh [DIR]    # DIR: where to copy (default .robigo/oracle-lib)
set -euo pipefail
cd "$(dirname "$0")/.."
dir=${1:-.robigo/oracle-lib}
mkdir -p "$dir"
dir=$(cd "$dir" && pwd)
docker image inspect robigo-oracle:4.6.2 > /dev/null 2>&1 || {
  echo "error: no image robigo-oracle:4.6.2; run scripts/build-oracle.sh --image 4.6.2" >&2
  exit 1
}
docker run --rm -u "$(id -u):$(id -g)" -v "$dir:/out" -e LC_ALL=C -e R_DEFAULT_PACKAGES=NULL \
  robigo-oracle:4.6.2 sh -c '
  for p in base compiler tools; do
    mkdir -p /out/$p/R && cp "$(R RHOME)/library/$p/R/$p.rdb" "$(R RHOME)/library/$p/R/$p.rdx" /out/$p/R/
  done
  Rscript --vanilla -e "
    envs <- list()
    inh <- function(n) { if (is.null(envs[[n]])) envs[[n]] <<- new.env(); envs[[n]] }
    outh <- function(e) { for (n in names(envs)) if (identical(envs[[n]], e)) return(n); NULL }
    for (p in c(\"base\", \"compiler\", \"tools\")) {
      d <- file.path(\"/out\", p, \"R\")
      x <- readRDS(file.path(d, paste0(p, \".rdx\")))
      con <- file(file.path(d, paste0(p, \".rdb\")), \"rb\")
      out <- file(file.path(d, paste0(p, \".roundtrip\")), \"wb\")
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
    }"'
ROBIGO_ORACLE_LIBRARY=$dir cargo test --release -p robigo-rt --test rdb_roundtrip -- --nocapture
