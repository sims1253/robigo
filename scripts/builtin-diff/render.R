## Shows how two of A4's records differ (scripts/builtin-diff.py): read by any
## R >= 4.0, since it only reads and prints them; the comparison itself is of
## their bytes.
##   Rscript render.R LIST          # LIST: lines "id<TAB>gnur.rds<TAB>rt.rds"
##                                  # out:  "id<TAB>field<TAB>GNU R's<TAB>robigo-rt's"
##   Rscript render.R --full A B    # both records, field by field
local({
    args <- commandArgs(TRUE)
    rd <- function(f) tryCatch(readRDS(f), error = function(e) list(unreadable = conditionMessage(e)))
    short <- function(x) {
        s <- tryCatch(paste(deparse(x, width.cutoff = 100L, nlines = 4L), collapse = " "),
                      error = function(e) paste("<", class(x)[[1L]], ">"))
        s <- gsub("[\t\n]", " ", s)
        if (nchar(s, "bytes") > 200L) paste0(substr(s, 1L, 197L), "...") else s
    }
    same <- function(a, b) identical(serialize(a, NULL), serialize(b, NULL))
    first_diff <- function(a, b) {
        for (f in union(names(a), names(b))) if (!same(a[[f]], b[[f]])) return(f)
        ## every field the same once read back: the bytes differ in how they
        ## were written (a hashed environment's table, a reference's order)
        "(serialization only)"
    }
    if (identical(args[[1L]], "--full")) {
        a <- rd(args[[2L]]); b <- rd(args[[3L]])
        for (f in union(names(a), names(b))) {
            cat(if (same(a[[f]], b[[f]])) "  " else "* ", f, "\n", sep = "")
            if (!same(a[[f]], b[[f]])) {
                cat("  GNU R:\n"); print(a[[f]])
                cat("  robigo-rt:\n"); print(b[[f]])
            }
        }
        return(invisible())
    }
    for (line in readLines(args[[1L]])) {
        p <- strsplit(line, "\t", fixed = TRUE)[[1L]]
        a <- rd(p[[2L]]); b <- rd(p[[3L]])
        f <- first_diff(a, b)
        cat(p[[1L]], "\t", f, "\t", short(a[[f]]), "\t", short(b[[f]]), "\n", sep = "")
    }
})
