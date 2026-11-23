# base's lapply and vapply evaluate one FUN(X[[i]], ...) call per isolate,
# kept across invocations: what R code can see of it is the same as GNU R's.
f <- function(v) sys.call()
a <- lapply(1:2, f)
b <- lapply(3:4, f)
a[[1]]
identical(a[[1]], b[[2]])
# a change to a retrieved call copies it: the next invocation's is intact
g <- function(v) {
  sc <- sys.call()
  sc[[1]] <- quote(h)
  sc[[2]][[1]] <- quote(`[`)
  sc[[3]] <- 99
  sc
}
lapply(1, g)
lapply(1, f)
vapply(1:2, function(v) deparse(sys.call()), "")
lapply(1, f)
# lapply inside FUN inside lapply: the same call, evaluated recursively
inner <- function(v) unlist(lapply(1:3, function(w) v * 10 + w))
lapply(1:2, inner)
k <- function(v) list(sys.call(), lapply(v, function(w) sys.call()))
lapply(1:2, k)
# an error's call
tryCatch(lapply(1, function(v) stop("no")), error = function(e) conditionCall(e))
# .Internal(lapply()) of another closure: its own call
my <- function(Y, G, ...) .Internal(lapply(Y, G))
my(1:2, function(v) sys.call())
my2 <- function(X, FUN, ...) {
  `[[` <- function(x, i) -i
  .Internal(lapply(X, FUN))
}
my2(1:3, identity)
lapply(1:3, identity)
# vapply's own call, and sapply through lapply
vapply(1:3, function(v) v + 0.5, 0)
sapply(1:3, function(v) v * 2)
sapply(list(a = 1, b = 2), f)
# a closure of the base namespace with a `[[` of its own: its own call
invisible(lapply(1:2, identity))
my3 <- function(X, FUN, ...) {
  `[[` <- function(x, i) -i
  .Internal(lapply(X, FUN))
}
environment(my3) <- environment(lapply)
unlist(my3(11:13, identity))
unlist(lapply(11:13, identity))
invisible(vapply(1:2, identity, 0L))
my4 <- function(X, FUN, FUN.VALUE, ..., USE.NAMES = TRUE) {
  `[[` <- function(x, i) -i
  .Internal(vapply(X, FUN, FUN.VALUE, USE.NAMES))
}
environment(my4) <- environment(vapply)
my4(11:13, identity, 0L)
vapply(11:13, identity, 0L)
# what FUN sees of its call
lapply(1:2, function(v) substitute(v))
lapply(1, function(v, ...) match.call())
lapply(1, function(v) deparse(sys.function()))
