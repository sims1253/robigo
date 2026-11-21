#!/usr/bin/env bash
# L1's bar (m2m3-plan §B L1): GNU R 4.6.2's own lazy-load databases (base,
# compiler and tools, from the oracle image robigo-oracle:4.6.2) unserialize
# record by record on robigo-rt and serialize back to the same bytes, the
# records holding one of base's compact sequences (ALTREP, robigo-rtcore's
# altrep::own) included. Runs crates/robigo-rt/tests/rdb_roundtrip.rs on a copy of the
# image's library, against GNU R's own round trip of each record (written
# beside it as PKG.roundtrip by GNU R in the image): GNU R's reader changes
# a few records (a forced promise's PRENV, R_NilValue, becomes the base
# environment, serialize.c's ReadItem_Iterative), so its re-serialization of
# those is not their bytes either, and robigo-rt's must be GNU R's.
# Then crates/robigo-rtapi/tests/lazyload.rs: base's own lazyLoad of the same
# databases on robigo-rt (makeLazy, lazyLoadDBfetch) against GNU R's lazyLoad
# of them (PKG.lazyload, written beside by GNU R in the image).
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
  set -eu
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
    }"
  Rscript --vanilla -e "
    # GNU R'"'"'s own lazyLoad of each database (PKG.lazyload): per variable, in
    # sorted order, a checksum of its serialization (version 2, after the
    # header) and one with every closure in it (itself, in a list or an
    # environment) replaced by its formals, body and attributes, and the
    # top-level environment by its sorted contents (nested ones by a mark): closures and environments
    # enclosures aside (tests/lazyload.rs computes the same two)
    ck <- function(x) { b <- as.numeric(as.integer(serialize(x, NULL, version = 2L))[-(1:14)])
                        paste(length(b), sum(b * seq_along(b)) %% 1000000007) }
    nz <- function(x, top = TRUE) if (is.function(x) && !is.primitive(x)) list(formals(x), body(x), attributes(x)) else
                      if (is.environment(x)) { if (top) list(\"env\", nz(as.list(x, all.names = TRUE, sorted = TRUE), FALSE)) else \"env\" } else
                      if (is.list(x)) { a <- attributes(x); r <- lapply(x, nz, top = top); attributes(r) <- a; r } else x
    for (p in c(\"base\", \"compiler\", \"tools\")) {
      d <- file.path(Sys.getenv(\"OUT\"), p, \"R\")
      e <- new.env()
      lazyLoad(file.path(d, p), envir = e)
      lines <- vapply(sort(ls(e, all.names = TRUE)), function(n) {
        x <- get(n, envir = e)
        paste(n, ck(x), ck(nz(x)))
      }, \"\")
      f <- file.path(d, paste0(p, \".lazyload\"))
      writeLines(lines, paste0(f, \".tmp\"))
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
# both tests run, each reported; the script fails if either does
status=0
ROBIGO_ORACLE_LIBRARY=$dir cargo test --release -p robigo-rt --test rdb_roundtrip -- --nocapture || status=1
ROBIGO_ORACLE_LIBRARY=$dir cargo test --release -p robigo-rtapi --test lazyload -- --nocapture || status=1
[ "$status" = 0 ] && echo "rt-rdb-roundtrip: both tests passed" || echo "rt-rdb-roundtrip: a test failed (above)" >&2
exit "$status"
