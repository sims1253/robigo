# MAKECLOSURE's constant list(formals, body, srcref): a non-NULL third
# element becomes the new closure's "srcref" attribute, that object itself,
# whatever it is (eval.c:bcEval MAKECLOSURE; setAttrib does not check it).
# robigo-rt's translation hands it on as Op::MkClosure's `b`, a constant of
# the translated code's pool. The values here are GNU R 4.6.2's. An index
# past the pool is malformed byte code. scripts/rt-bc-mutate.py runs it
# once; it prints "makeclosure ok". (Not a conformance case: no expression
# compiles to this byte code.)
# MAKECLOSURE 0, RETURN
mk <- function(fb, i = 0L) .Internal(bcClose(NULL, .Internal(mkCode(c(12L, 41L, i, 1L), list(fb))), globalenv()))
# a srcref of its own text: the closure prints from it
sf <- srcfilecopy("<mkc>", "function(x) x + 1  # kept")
sr <- srcref(sf, c(1L, 1L, 1L, 25L))
g1 <- mk(list(as.pairlist(alist(x = )), quote(x + 1), sr))()
stopifnot(identical(attr(g1, "srcref"), sr), identical(attr(attr(g1, "srcref"), "srcfile"), sf), g1(1) == 2,
          identical(deparse(g1, control = "useSource"), "function(x) x + 1  # kept"))
# a third element that is no srcref: the attribute all the same
g2 <- mk(list(as.pairlist(alist(x = )), quote(x * 2), 42L))()
stopifnot(identical(attr(g2, "srcref"), 42L), g2(3) == 6)
# NULL, and none: no attribute
g3 <- mk(list(as.pairlist(alist(x = )), quote(x), NULL))()
g4 <- mk(list(as.pairlist(alist(x = )), quote(x)))()
stopifnot(is.null(attributes(g3)), is.null(attributes(g4)))
# the constant's index past the pool
m <- tryCatch(mk(list(as.pairlist(alist(x = )), quote(x), sr), 5L)(), error = function(e) conditionMessage(e))
stopifnot(startsWith(m, "robigo: malformed byte code"))
cat("makeclosure ok\n")
