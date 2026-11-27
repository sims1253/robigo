# relop.c do_relop_dflt with a list or expression operand (#1032):
# coerceVector ladder (STRSXP, CPLXSXP, REALSXP, INTSXP, LGLSXP, RAWSXP),
# zero-length short-circuit, attribute preservation, warnings/errors order.
probe <- function(lab, expr) {
  cat("--", lab, "\n")
  r <- withCallingHandlers(
    tryCatch(expr, error = function(e) { cat("error:", conditionMessage(e), "\n"); invisible(NULL) }),
    warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }
  )
  if (!is.null(r)) print(r)
}

# Issue #1032 motivating cases
probe("list(2) > 0", list(2) > 0)
probe("list(2) >= 3", list(2) >= 3)
probe("0 < list(2)", 0 < list(2))

# Numeric / integer / logical ladder (note list(1.5) == 1L truncates to INTSXP)
probe("list(1, 2, 3) == c(1, 2, 4)", list(1, 2, 3) == c(1, 2, 4))
probe("list(1.5) == 1L", list(1.5) == 1L)
probe("list(1.5) == 1.0", list(1.5) == 1.0)
probe("list(2, 0, NA) == TRUE", list(2, 0, NA) == TRUE)
probe("list('abc') == 1.0", list("abc") == 1.0)

# String / symbol ladder (deparses non-scalar / non-atomic list & expression elements)
probe("list(1:2, 'a') == c('1:2', 'a')", list(1:2, "a") == c("1:2", "a"))
probe("list('a', 'b') == quote(a)", list("a", "b") == quote(a))
probe("expression(x + 1) == 'x + 1'", expression(x + 1) == "x + 1")

# Complex ladder (coerces BEFORE checking invalid ordering on complex)
probe("list(1, 2+1i) == c(1+0i, 2+1i)", list(1, 2 + 1i) == c(1 + 0i, 2 + 1i))
probe("list(1) < 1i", list(1) < 1i)
probe("list(1:2) < 1i", list(1:2) < 1i)

# Raw ladder
probe("list(1, 255) == as.raw(c(1, 255))", list(1, 255) == as.raw(c(1, 255)))
probe("list(300) == as.raw(0)", list(300) == as.raw(0))

# Both non-empty lists / expressions -> not implemented (GNU R's own error)
probe("list(1) == list(1)", list(1) == list(1))
probe("expression(1) == expression(1)", expression(1) == expression(1))
probe("list(1) == expression(1)", list(1) == expression(1))

# Non-vectorizable list / expression vs atomic
probe("list(1:2) > 0", list(1:2) > 0)
probe("expression(1) > 0", expression(1) > 0)

# Zero-length short-circuit (no coercion, no error even for non-vectorizable/list==list)
probe("list() == list()", list() == list())
probe("list(1:2) == integer(0)", list(1:2) == integer(0))
probe("list() == 1", list() == 1)
probe("expression() == 1", expression() == 1)
probe("list(1) == NULL", list(1) == NULL)

# Length mismatch warning fires BEFORE coercion error
probe("mismatch before coerce err", list(1, 1:2) == c(1, 2, 3))

# Attributes (names / dim) preserved from original list operand
l <- list(a = 1, b = 2)
probe("named list == 1:2", l == 1:2)
m <- matrix(as.list(1:4), 2, 2, dimnames = list(c("r1", "r2"), c("c1", "c2")))
probe("list matrix > 2", m > 2)
