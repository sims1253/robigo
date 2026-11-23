# A local bound to `a:b` is one object: reading it, putting it in a list
# and reading the list element all see the same object, as in GNU R
# (data.table's .onLoad stops with "list(x) has copied x" otherwise).
addr <- function(v) {
    tc <- textConnection("out", "w", local = TRUE)
    sink(tc)
    .Internal(inspect(v))
    sink()
    close(tc)
    sub(" .*", "", out[1L])
}
f <- function() {
    x <- 1L:3L
    y <- list(x)
    c(same_list = addr(x) == addr(y[[1L]]), same_twice = addr(x) == addr(x))
}
print(f())
g <- function(n) {
    x <- seq_len(n)
    z <- x
    y <- list(x, z)
    c(addr(x) == addr(z), addr(y[[1L]]) == addr(y[[2L]]))
}
print(g(5L))
# A range variable still reads and indexes as an integer vector.
h <- function() {
    x <- 2L:6L
    s <- 0L
    for (i in seq_along(x)) s <- s + x[i]
    x[2L] <- 10L
    list(s, x, typeof(x))
}
print(h())
