# DispatchOrEval's promiseArgs for `[` methods (eval.c): missing arguments,
# tags and `...` reach the method as GNU R gives them, and a promise the
# method keeps survives R_CleanupEnvir and unpromiseArgs. From byte code
# (a loop compiles f) and from the AST.
`[.pa` <- function(x, i, j, ..., drop = TRUE) {
    list(mi = missing(i), mj = missing(j), md = missing(drop),
         si = if (!missing(i)) deparse(substitute(i)),
         mc = deparse(match.call(expand.dots = FALSE)), nd = ...length())
}
x <- structure(list(), class = "pa")
v <- 7
f <- function() list(x[], x[, drop = FALSE], x[v + 1, , 3])
fw <- function(...) x[...]
g <- function() list(fw(), fw(v, j = 2), fw(, , drop = FALSE))
for (k in 1:3) { r <- f(); s <- g() }
print(r)
print(s)
print(eval(quote(list(x[], x[v, drop = FALSE]))))
# a method keeps a promise of its argument and forces it later
`[.keep` <- function(x, i) function() i
y <- structure(list(), class = "keep")
n <- 0
h <- function() y[{ n <<- n + 1; n * 10 }]
for (k in 1:3) th <- h()
print(n)
print(th())
print(th())
print(n)
# NextMethod from a `[` method
`[.nm` <- function(x, i) { r <- NextMethod(); paste("nm", unclass(r)) }
z <- structure(c("a", "b", "c"), class = "nm")
q <- function(w) z[w]
for (k in 1:3) print(q(2:3))
print(z[1])
