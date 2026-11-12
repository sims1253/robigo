# Argument checks and argument handling that must match GNU R's C code.
show <- function(expr) {
  withCallingHandlers(
    tryCatch({ v <- expr; print(v) }, error = function(e) {
      cat("Error: "); print(conditionCall(e)); cat("  ", conditionMessage(e), "\n")
    }),
    warning = function(w) {
      cat("Warning: "); print(conditionCall(w)); cat("  ", conditionMessage(w), "\n")
      invokeRestart("muffleWarning")
    })
}

# R: debug.c:do_trace — errorcall() with the primitive's own call
show(.primUntrace(42))
show(.primTrace(list()))
f <- function() NULL
show(.primTrace(f)); show(.primUntrace(f))

# R: array.c:do_rowscols — coerceVector() warns, allocMatrix() checks
show(.Internal(row("str")))
show(.Internal(col(c("2", "x"))))
show(.Internal(row(c(2L, 3L))))
show(.Internal(col(c(NA, 3L))))

# R: unique.c:do_pmatch — asInteger(nomatch) warns
show(pmatch(1:5, c(1, 3), nomatch = "str"))
show(pmatch("a", "b", nomatch = 2^40))

# R: main.c:do_quit validates its arguments before quitting
show(quit("xx"))
show(quit(1))
show(quit(character(0)))

# R: eval.c:CheckFormals — error(), so the context call is reported
show(eval(call("function", 1, quote(x + 1))))
show(eval(call("function", as.pairlist(list(1)), quote(x + 1))))
g <- function() eval(call("function", "a", quote(a)))
show(g())

# R: subset.c:R_DispatchOrEvalSP conses a non-object untagged (CONS_NR):
# a `drop =` tag on it is not the drop argument
m <- matrix(1:6, nrow = 2, dimnames = list(c("a", "b"), LETTERS[1:3]))
show(`[`(drop = m, 1, , drop = FALSE))
show(`[`(drop = m, 1, ))
show(`[[`(exact = list(abc = 1), "a", exact = FALSE))

# R: relop.c — an empty operand gives logical(0) before any type check
show(list() >= list())
show(list(1) >= list())
show(list() == expression())
show(structure(list(), names = character(0)) < list())
show(list(1) >= list(1))
k <- function(x, y) x >= y
show(k(list(), list())); show(k(list(), list()))
x <- structure(list(c0 = structure(integer(0), .Label = character(0), class = "factor")),
               .Names = "c0", row.names = character(0),
               class = structure("integer(0)", .Names = "c0"))
show(.Internal(is.unsorted(x, FALSE)))
show(is.unsorted(list(1, 2)))
