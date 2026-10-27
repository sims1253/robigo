# Download the source tarballs of the installed packages into a directory.
# Usage: Rscript --vanilla scripts/parse-cran-download.R DESTDIR LIBDIR
# The tarballs are only ever parsed (see scripts/parse-corpus.py), never
# installed or run.

args <- commandArgs(trailingOnly = TRUE)
dest <- normalizePath(args[[1L]], mustWork = FALSE)
dir.create(dest, recursive = TRUE, showWarnings = FALSE)
lib <- args[[2L]]
options(repos = c(CRAN = "https://cloud.r-project.org"), timeout = 600)

pkgs <- rownames(installed.packages(lib.loc = lib))
pkgs <- setdiff(pkgs, rownames(installed.packages(lib.loc = .Library)))
have <- list.files(dest, pattern = "\\.tar\\.gz$")
want <- pkgs[!vapply(pkgs, function(p) any(startsWith(have, paste0(p, "_"))), logical(1))]
cat(length(pkgs), "packages,", length(want), "to fetch\n")
if (length(want)) {
    ok <- available.packages()
    miss <- setdiff(want, rownames(ok))
    if (length(miss)) cat("not on CRAN (skipped):", paste(miss, collapse = " "), "\n")
    want <- intersect(want, rownames(ok))
    if (length(want))
        download.packages(want, destdir = dest, type = "source", quiet = TRUE)
}
