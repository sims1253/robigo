# R_decompress1 (libdeflate) on corrupted records of a lazy-load database
# (data/corrupt.rdb, written by data/make-corrupt-rdb.R on GNU R 4.6.2):
# libdeflate's result in the warning and lazyLoadDBfetch's error for bad
# data (1) and for a header that promises too few bytes (3,
# LIBDEFLATE_INSUFFICIENT_SPACE); a header that promises too many is no
# error: the stream's own bytes (actual_out), as GNU R takes them.
d <- "data"
for (up in c("", "../", "../../", "../../../", "../../../../", "../../../../../"))
    if (!dir.exists(d)) d <- paste0(up, "tests/rt/serialize/data")
f <- file.path(d, "corrupt.rdb")
fetch <- function(key) {
    r <- tryCatch(withCallingHandlers(lazyLoadDBfetch(key, f, TRUE, NULL),
                                      warning = function(w) {
                                          cat("warning:", conditionMessage(w), "\n")
                                          invokeRestart("muffleWarning")
                                      }),
                  error = function(e) paste("error:", sub(f, "FILE", conditionMessage(e), fixed = TRUE)))
    print(r)
}
fetch(c(0L, 132L))    # good
fetch(c(132L, 26L))   # bad data
fetch(c(158L, 132L))  # too few
fetch(c(290L, 132L))  # too many
fetch(c(0L, 132L))    # good again: the decompressor serves the next record
