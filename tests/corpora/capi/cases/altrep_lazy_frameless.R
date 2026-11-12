dyn.load("altrep_lazy_string.so")
lazy <- function(fn) .Call("lazy_new", fn)
mat <- function(x) .Call("lazy_is_materialized", x)
heads <- function() {
    h <- vapply(sys.calls(), function(x) if (is.name(x[[1L]])) as.character(x[[1L]]) else "<fn>", "")
    cat(h, "\n")
}

# A package ALTREP vector whose materialization runs R code, read by the
# internal of a simple .Internal wrapper (nchar, rep.int, rep_len) called as
# a closure: the R code runs once, inside the wrapper's frame, and its
# conditions reach the handlers (#514).
z <- lazy(function() { cat("materialize\n"); warning("lazy warning"); "abc" })
withCallingHandlers(print(nchar(z)), warning = function(w) {
    cat("caught warning:", conditionMessage(w), "\n")
    invokeRestart("muffleWarning")
})
print(mat(z))
print(nchar(z))

# output before an error in the materializer comes once
e <- lazy(function() { cat("once\n"); stop("lazy boom") })
print(tryCatch(nchar(e), error = function(err) conditionMessage(err)))
print(mat(e))

# the materializer sees the wrapper's frame
f <- lazy(function() { heads(); "xy" })
nchar(f)
m <- lazy(function() { heads(); c("p", "q") })
rep.int(m, 2L)

# through length() in a wrapper's argument
g <- lazy(function() { cat("length\n"); heads(); c("a", "b") })
rep.int(1L, length(g))
k <- lazy(function() { warning("k warned"); "w" })
withCallingHandlers(rep_len(0L, length(k)), warning = function(w) {
    cat("caught:", conditionMessage(w), "\n")
    invokeRestart("muffleWarning")
})
