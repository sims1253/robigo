# R: lazyload.R:lazyLoadDBexec. Enumerate both object and reference keys;
# forcing bindings misses entries in packages that cannot yet initialize.
library_dir <- Sys.getenv("ROBIGO_RDBFLAT_LIBRARY")
if (!nzchar(library_dir)) library_dir <- file.path(R.home(), "library")
indices <- list.files(library_dir, pattern = "\\.rdx$", recursive = TRUE,
                      full.names = TRUE)
stopifnot(length(indices) > 0L)
emit_key <- function(key, database, compressed) {
    # R: lazyload.R:lazyLoadDBexec also supports partially lazy objects.
    if (is.list(key)) {
        stopifnot(length(key$eagerKey) == 2L, is.list(key$lazyKeys))
        emit_key(key$eagerKey, database, compressed)
        for (lazy_key in key$lazyKeys) emit_key(lazy_key, database, compressed)
    } else {
        stopifnot(is.numeric(key), length(key) == 2L,
                  !anyNA(key), all(key >= 0), all(key == as.integer(key)))
        cat(database, key[1L], key[2L], as.integer(compressed), sep = "\t")
        cat("\n")
    }
}
for (index_file in indices) {
    index <- readRDS(index_file)
    stopifnot(is.list(index), is.list(index$variables),
              is.list(index$references), length(index$compressed) == 1L)
    database <- sub("\\.rdx$", ".rdb", index_file)
    stopifnot(file.exists(database))
    for (key in c(index$variables, index$references))
        emit_key(key, database, index$compressed)
}
