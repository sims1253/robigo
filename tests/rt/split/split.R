# split.c do_split (through base's split.default and directly): each level's
# elements in order, with their names, pieces of x's type with x's levels
# attribute, the result named by the factor's levels.
show <- function(expr) {
  r <- tryCatch(expr, error = function(e) e)
  if (inherits(r, "error")) {
    cat("error:", conditionMessage(r), "| call:", deparse(conditionCall(r)), "\n")
    return(invisible())
  }
  cat("list of", length(r), if (!is.null(names(r))) paste("names:", paste(names(r), collapse = ",")), "\n")
  for (i in seq_along(r)) {
    v <- r[[i]]
    cat(" [[", i, "]]", typeof(v), ":", if (is.list(v)) "<list>" else v, "\n")
    a <- attributes(v)
    for (n in names(a)) cat("    ", n, ":", a[[n]], "\n")
  }
  invisible()
}
f <- factor(c("a", "b", "a", "c", "b"))
show(split(1:5, f))
show(split(c(x = 1.5, y = 2, z = 3, w = 4, v = 5), f))
show(split(letters[1:5], f))
show(split(c(TRUE, FALSE, NA, TRUE, FALSE), f))
show(split(c(1i, 2, 3, 4, 5), f))
show(split(as.raw(1:5), f))
show(split(list(1, "a", NULL, 2:3, sum), f))
# recycling the factor, and the warning when lengths do not divide
show(split(1:6, factor(c("p", "q"))))
show(withCallingHandlers(split(1:5, factor(c("p", "q"))),
  warning = function(w) { cat("warning:", conditionMessage(w), "| call:", deparse(conditionCall(w)), "\n"); invokeRestart("muffleWarning") }))
# NA levels are dropped; unused levels give empty pieces; drop
g <- factor(c("a", NA, "b", "a"), levels = c("a", "b", "z"))
show(split(1:4, g))
show(split(1:4, g, drop = TRUE))
# x's levels attribute goes on each piece; a classed x goes through [ instead
lx <- structure(1:4, levels = c("L1", "L2"))
show(split(lx, factor(c(1, 2, 1, 2))))
show(split(factor(c("u", "v", "u", "w")), factor(c(1, 1, 2, 2))))
# a compact sequence, as split.default makes for a classed x
show(.Internal(split(seq_along(letters[1:6]), factor(rep(c("e", "o"), 3)))))
show(.Internal(split(1:6, factor(rep(c("e", "o"), 3)))))
# empty and zero-length
show(split(integer(), factor(character())))
show(split(integer(), factor(c("a", "b"))))
show(.Internal(split(1:3, factor(character()))))
# errors
show(.Internal(split(sum, factor("a"))))
show(.Internal(split(1:3, c(1L, 2L, 1L))))
show(.Internal(split(1:3, structure(c(1L, 5L, 1L), levels = c("a", "b"), class = "factor"))))
show(.Internal(split(1:3, structure(c(1L, 0L, 1L), levels = c("a", "b"), class = "factor"))))
show(.Internal(split(expression(1, 2), factor(c("a", "b")))))
show(.Internal(split(expression(1, 2), structure(c(NA_integer_, NA_integer_), levels = "a", class = "factor"))))
show(.Internal(split(1:3)))
# the call base's code makes, as in tools' R_license_db_vars
pos <- c(a = 1L, b = 2L, a = 3L)
show(split(pos, names(pos)))
