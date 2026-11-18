# envir.c's .Internals over `...` with an explicit environment and
# `inherits`: dotsExist, getDotType, dotDelayedExpression,
# dotDelayedEnvironment, dotForcedExpression, dotsLength, dotsNames, dotsElt.
show <- function(expr) {
  r <- tryCatch(expr, error = function(e) e)
  if (inherits(r, "error")) {
    cat("error:", conditionMessage(r), "| call:", deparse(conditionCall(r)), "\n")
  } else print(r)
  invisible()
}
f <- function(...) {
  e <- environment()
  list(exist = .Internal(dotsExist(e, FALSE)),
       len = .Internal(dotsLength(e, FALSE)),
       names = .Internal(dotsNames(e, FALSE)),
       types = vapply(seq_len(...length()), function(i) .Internal(getDotType(i, e, FALSE)), ""))
}
show(f())
show(f(1, b = x + y, , 3))
# a compiled caller passes a constant as a value (PUSHCONSTARG)
cf <- function() { for (i in 1) NULL; f(1, b = x + y, , 3)$types }
show(cf())
show(cf())
# delayed, forced, value and missing elements
g <- function(...) {
  e <- environment()
  force(..1)
  list(.Internal(getDotType(1L, e, FALSE)),
       .Internal(dotForcedExpression(1L, e, FALSE)),
       .Internal(getDotType(2L, e, FALSE)),
       .Internal(dotDelayedExpression(2L, e, FALSE)),
       identical(.Internal(dotDelayedEnvironment(2L, e, FALSE)), globalenv()),
       .Internal(getDotType(3L, e, FALSE)))
}
show(g(1 + 1, a * b, ))
# the errors of the expression accessors
h <- function(...) {
  e <- environment()
  force(..1)
  for (k in list(
    quote(.Internal(dotDelayedExpression(1L, e, FALSE))),
    quote(.Internal(dotDelayedEnvironment(1L, e, FALSE))),
    quote(.Internal(dotForcedExpression(2L, e, FALSE))),
    quote(.Internal(dotDelayedExpression(3L, e, FALSE))),
    quote(.Internal(dotForcedExpression(3L, e, FALSE))),
    quote(.Internal(dotDelayedExpression(9L, e, FALSE))),
    quote(.Internal(dotDelayedExpression(0L, e, FALSE))),
    quote(.Internal(getDotType(-1L, e, FALSE)))))
    show(eval(k))
}
h(1, q, )
# inherits: the first frame outwards with `...`
outer <- function(...) { inner <- function() .Internal(dotsLength(environment(), TRUE)); inner() }
show(outer(1, 2, 3))
show(.Internal(dotsExist(globalenv(), TRUE)))
show(.Internal(dotsExist(globalenv(), FALSE)))
show(.Internal(dotsLength(globalenv(), FALSE)))
show(.Internal(dotsNames(globalenv(), TRUE)))
show(.Internal(dotsElt(1L, globalenv(), TRUE)))
show(.Internal(getDotType(1L, globalenv(), FALSE)))
# dotsElt evaluates a promise or the missing argument
k <- function(...) {
  e <- environment()
  c(.Internal(dotsElt(1L, e, FALSE)), .Internal(dotsElt(2, e, FALSE)))
}
show(k(10, 5 * 4))
m <- function(...) .Internal(dotsElt(1L, environment(), FALSE))
show(m())
show(m(, 2))
# a forwarded `...`: the chain's innermost promise
p <- function(...) q2(...)
q2 <- function(...) .Internal(dotDelayedExpression(1L, environment(), FALSE))
show(p(u + v))
# arguments that are not what is expected
r <- function(...) list(.Internal(dotsLength(environment(), NA)), .Internal(getDotType("1", environment(), FALSE)))
show(r(1))
# an active `...` binding: the index is checked before it runs, and each
# read of the frame runs it
cnt <- 0
ae <- new.env()
makeActiveBinding("...", function() { cnt <<- cnt + 1; NULL }, ae)
show(.Internal(getDotType(0L, ae, FALSE)))
cnt
show(.Internal(getDotType(1L, ae, FALSE)))
cnt
show(.Internal(dotsLength(ae, FALSE)))
cnt
show(.Internal(dotsExist(ae, FALSE)))
cnt
show(.Internal(dotsExist(ae, TRUE)))
cnt
