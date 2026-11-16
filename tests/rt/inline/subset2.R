# `x[[i]]` and `is.null(x)` in inline runs: a plain list read at an
# in-range numeric index, and NULL tested, as GNU R's VECSUBSET2 and
# is.null leave them. Every other case runs as GNU R's default does: its
# values, errors and warnings, with their calls. Each case runs three
# times from a compiled closure.
check <- function(t) if (is.null(t[[1]])) 1 else 1 + check(t[[1]]) + check(t[[2]])
make <- function(d) if (d == 0) list(NULL, NULL) else list(make(d - 1), make(d - 1))
r1 <- function() c(check(make(3)), check(make(5)), check(list(NULL, 1)))
for (k in 1:3) print(r1())
e1 <- function(x) x[[1]]
e2 <- function(x) x[[2]]
el <- function(x, i) x[[i]]
cond <- function(e) list(conditionMessage(e), conditionCall(e))
l <- list(10, "b", NULL, list(1))
# indices: integer, double, a fraction, the last, past the end, zero,
# negative, NA
r2 <- function() list(e1(l), e2(l), el(l, 2L), el(l, 2.9), el(l, 4), el(l, NA_real_), el(list(1, 2), -1))
for (k in 1:3) print(r2())
r3 <- function() list(tryCatch(el(l, 5), error = cond), tryCatch(el(l, 0), error = cond),
                      tryCatch(el(l, -1), error = cond), tryCatch(el(l, "a"), error = cond))
for (k in 1:3) print(r3())
# other objects: an atomic vector, names, a pairlist, NULL, an environment,
# a class with a method
v <- c(a = 1, b = 2)
en <- new.env(); assign("z", 5, envir = en)
`[[.box` <- function(x, i) paste("box", i)
b <- structure(list(1, 2), class = "box")
r4 <- function() list(e2(v), e1(list(a = 1)), e1(pairlist(7, 8)), e1(NULL), el(en, "z"), e2(b))
for (k in 1:3) print(r4())
# is.null of NULL, an empty list, a number, NA
isn <- function(x) is.null(x)
r5 <- function() c(isn(NULL), isn(list()), isn(0), isn(NA), isn(e1(list(NULL))))
for (k in 1:3) print(r5())
# an element taken out is shared with its list: changing it copies
r6 <- function() { m <- list(c(1, 2)); e <- e1(m); e[1] <- 9; list(e, m[[1]]) }
for (k in 1:3) print(r6())
# visibility at top level through a compiled caller
vis <- function() e1(list(5))
vis()
vinv <- function() invisible(e1(list(6)))
vinv()
# `[[` and is.null rebound: their guards send the calls to the new ones
r7 <- function() c(e1(list(3)), isn(NULL))
for (k in 1:3) print(r7())
`[[` <- function(x, i) "mine"
is.null <- function(x) "also mine"
for (k in 1:3) print(r7())
rm(`[[`, is.null)
for (k in 1:3) print(r7())
