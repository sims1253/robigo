# Observe GNU R's visible compilation state, without process addresses.
show <- function(f) {
    path <- tempfile()
    sink(path)
    print(f)
    sink()
    lines <- readLines(path)
    unlink(path)
    cat(gsub("0x[0-9a-f]+", "0x", lines), sep = "\n")
    cat("\n")
}
f <- function(x) x + 1L
show(f)
f(1L)
show(f)
f(1L)
show(f)
# Base closures arrive already byte-compiled from the lazy-load database.
show(match)
# Small closures in a non-global environment do not reach the JIT score.
e <- new.env()
g <- eval(quote(function(x) x + 2L), e)
g(1L)
g(1L)
show(g)
