# for (j in seq(from, to, by)) with integer arguments: GNU R's seq.default
# result, its checks, and dispatch that may leave seq.default. (The VM
# reads the range off; everything else makes the call.)
f <- function(a, b, by) {
    r <- integer()
    for (j in seq(a, b, by)) r <- c(r, j)
    list(r, typeof(r), if (exists("j", inherits = FALSE)) j else NA)
}
str1 <- function(x) cat(deparse(x), "\n")
str1(f(5L, 1L, -1L))
str1(f(1L, 10L, 3L))
str1(f(1L, 10L, 1L))
str1(f(-3L, 3L, 2L))
str1(f(4L, 4L, 1L))
str1(f(4L, 4L, -7L))
str1(f(0L, 0L, 5L))
str1(f(4L, 4L, 0L))
str1(f(.Machine$integer.max - 2L, .Machine$integer.max, 1L))
str1(f(-.Machine$integer.max, -.Machine$integer.max + 3L, 2L))
# Doubles, logicals and mixtures go through seq.default.
str1(f(1, 5, 2))
str1(f(1L, 5, 2L))
str1(f(1L, 5L, 0.5))
str1(f(TRUE, 3L, 1L))
str1(f(1L, TRUE, 1L))
str1(f(0.5, 3L, 1L))
# Errors and their calls.
for (args in list(list(1L, 5L, 0L), list(1L, 5L, -1L), list(5L, 1L, 1L), list(NA_integer_, 5L, 1L),
                  list(1L, NA_integer_, 1L), list(1L, 5L, NA_integer_), list(1L, 5L, c(1L, 2L)),
                  list(c(1L, 2L), 5L, 1L), list(-.Machine$integer.max, .Machine$integer.max, 1L)))
    print(tryCatch(do.call(f, args), error = function(e) conditionMessage(e)))
g <- function(n) {
    s <- 0L
    for (j in seq(n, 1L, -1L)) s <- s + j
    s
}
print(tryCatch(g(0L), error = function(e) conditionCall(e)))
# Arithmetic arguments, as in the shootout's pidigits.
mul <- function(x, y) {
    out <- integer(length(x) + length(y))
    x_len <- length(x)
    y_len <- length(y)
    for (i in seq(x_len - 1L, 1L, -1L)) {
        for (j in seq(y_len, 1L, -1L)) out[[i + j]] <- out[[i + j]] + x[[i]] * y[[j]]
    }
    out
}
print(mul(c(1L, 2L, 3L), c(4L, 5L)))
print(mul(c(7L, 8L), 9L))
h <- function(n) {
    k <- 0L
    for (j in seq(n + 1L, n + 10L, +2L)) k <- k + j
    for (j in seq(-n, n, 1L)) k <- k + j
    k
}
print(h(3L))
# Overflow in an argument: the warning once, then seq.default's error.
ov <- function(n) {
    for (j in seq(n + 1L, 1L, -1L)) NULL
    "done"
}
print(tryCatch(ov(.Machine$integer.max), error = function(e) conditionMessage(e),
               warning = function(w) conditionMessage(w)))
r <- withCallingHandlers(tryCatch(ov(.Machine$integer.max), error = function(e) conditionMessage(e)),
                         warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
print(r)
# Methods and rebindings.
seq.integer <- function(...) { cat("seq.integer\n"); 3:4 }
str1(f(1L, 5L, 1L))
rm(seq.integer)
seq.numeric <- function(...) { cat("seq.numeric\n"); 5:6 }
str1(f(1L, 5L, 1L))
rm(seq.numeric)
local({
    seq.default <- function(...) { cat("local seq.default\n"); 9L }
    k <- function() { s <- 0L; for (j in seq(1L, 5L, 1L)) s <- s + j; s }
    print(k())
})
w <- function() { seq <- function(...) 100:101; s <- 0L; for (j in seq(1L, 5L, 1L)) s <- s + j; s }
print(w())
m <- function(n) { `-` <- function(e1, e2) 1L; s <- 0L; for (j in seq(n - 1L, 1L, -1L)) s <- s + j; s }
print(m(5L))
p <- function(n) { `+` <- function(e1, e2) 7L; s <- 0L; for (j in seq(n + 1L, 20L, 1L)) s <- s + j; s }
print(p(5L))
# Promises: forced by seq.default, once, in order.
lazy <- function(a, b, by) { s <- 0L; for (j in seq(a, b, by)) s <- s + j; s }
print(lazy({ cat("a\n"); 1L }, { cat("b\n"); 4L }, { cat("by\n"); 1L }))
print(tryCatch(lazy(1L, 5L), error = function(e) conditionMessage(e)))
# Objects.
cls <- structure(1L, class = "myint")
seq.myint <- function(...) { cat("seq.myint\n"); 1:2 }
print(lazy(cls, 5L, 1L))
# Many calls, as in a loop.
tot <- 0L
for (k in 1:200) for (j in seq(k, 1L, -1L)) tot <- tot + j
print(tot)
# After many calls (the dispatch is memoized): a method in a frame, then
# one registered in base's methods table.
q <- function() {
    seq.integer <- function(...) { cat("frame seq.integer\n"); 7:8 }
    inner <- function() { s <- 0L; for (j in seq(1L, 3L, 1L)) s <- s + j; s }
    inner()
}
print(q())
print(lazy(1L, 3L, 1L))
registerS3method("seq", "integer", function(...) { cat("registered seq.integer\n"); 1:2 }, envir = baseenv())
print(lazy(1L, 5L, 1L))
