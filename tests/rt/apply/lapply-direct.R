# lapply's X[[i]] forced as GNU R forces it (robigo-rt computes the value
# directly when nothing of the force can be observed; these are the cases
# where something could be).
f <- function(v) substitute(v)
lapply(1:2, f)
lapply(1:2, function(v) sys.call())
lapply(1:2, function(v) missing(v))
lapply(1:3, function(v) 0)
# FUN rebinds X in lapply's frame: the next element reads the new X
lapply(1:3, function(v) {
  assign("X", c(10L, 20L, 30L), envir = parent.frame())
  v
})
# and `[[` there
lapply(1:3, function(v) {
  assign("[[", function(x, i) -i, envir = parent.frame())
  v
})
# and i
lapply(1:3, function(v) {
  assign("i", 1L, envir = parent.frame())
  v
})
# X with a class: [[ dispatches
x <- structure(list(1, 2), class = "myc")
`[[.myc` <- function(x, i) paste("elt", i)
lapply(x, identity)
x2 <- structure(1:3, class = "myd")
`[[.myd` <- function(x, i) i * 100
lapply(unclass(x2), identity)
sapply(x2, identity)
# a list element FUN modifies: the list is unchanged
l <- list(c(1, 2), c(3, 4))
r <- lapply(l, function(v) { v[1] <- 99; v })
l
r
l2 <- list(a = 1:3)
lapply(l2, function(v) { v[2] <- 0L; v })
l2
# the depth limit through lapply, as GNU R's
d <- 0
g <- function(n) { d <<- n; lapply(1, function(i) g(n + 1)) }
options(expressions = 500)
r <- tryCatch(g(1), error = function(e) conditionMessage(e))
r
d
options(expressions = 5000)
d <- 0
r <- tryCatch(g(1), error = function(e) conditionMessage(e))
r
d
# errors from FUN and from [[
tryCatch(lapply(1:2, function(v) stop("boom ", v)), error = function(e) conditionMessage(e))
tryCatch(lapply(pairlist(1, 2), function(v) v), error = function(e) conditionMessage(e))
vapply(1:3, function(v) v * 2, 0)
vapply(list(1, "a"), function(v) is.numeric(v), NA)
# a captured lapply call evaluated in another frame: its own X, i and FUN
cl <- lapply(1, function(v) sys.call())[[1]]
h <- function(FUN, X, i, ...) eval(cl)
h(function(v, ...) v * 2, c(5, 6, 7), 2L)
h(function(v, ...) list(v, ...), list("a", "b"), 1L, "extra")
k <- function(FUN, X, i, ...) {
  `[[` <- function(x, i) -i
  eval(cl)
}
k(identity, 1:3, 3L)
# a compact sequence with attributes, and one long enough to cross the
# evaluation count's polls many times
y <- 1:3
attr(y, "foo") <- "bar"
lapply(y, identity)
names(y) <- c("a", "b", "c")
lapply(y, function(v) v * 2L)
s <- 0
invisible(lapply(seq_len(3000L), function(v) s <<- s + v))
s
# lapply's index vector: captured by FUN (it must be fresh for the next
# element), rebound, or made an active binding in lapply's frame
kept <- list()
invisible(lapply(1:3, function(v) kept[[v]] <<- get("i", envir = parent.frame())))
unlist(kept)
lapply(1:3, function(v) { assign("i", 7L, envir = parent.frame()); v })
r <- lapply(1:3, function(v) {
  if (v == 1) {
    rm("i", envir = parent.frame())
    makeActiveBinding("i", function(x) if (missing(x)) 5L else invisible(NULL), parent.frame())
  }
  v
})
unlist(r)
