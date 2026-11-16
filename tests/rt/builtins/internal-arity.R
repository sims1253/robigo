# checkArity for an .Internal (names.c): "%d argument(s) passed to
# .Internal(%s) which requires %d", raised with error(), so against the
# calling closure's call.
m <- function(e) tryCatch(e, error = function(c) cat(conditionMessage(c), "\n"))
m(.Internal(gctorture(TRUE, 2)))
m(.Internal(gc(FALSE)))
m(.Internal(reg.finalizer(1)))
f <- function() .Internal(gctorture(TRUE, 2))
f()
