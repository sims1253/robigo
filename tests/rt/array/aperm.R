# array.c do_aperm: the permuted array, its dim's names and its dimnames
# (and their names) permuted with it, or only the old dim (resize = FALSE).
# Printed by attributes and elements (no printMatrix yet).
show <- function(x) {
  if (inherits(x, "error")) {
    cat("error:", conditionMessage(x), "| call:", deparse(conditionCall(x)), "\n")
    return(invisible())
  }
  cat(typeof(x), "|", format(x), "\n")
  a <- attributes(x)
  for (n in names(a)) { cat(" ", n, ": "); str1(a[[n]]) }
  invisible()
}
str1 <- function(v) {
  if (is.list(v)) {
    cat("list(", paste(names(v), vapply(v, function(e) paste(format(e), collapse = " "), ""), sep = "=", collapse = "; "), ")\n")
  } else cat(paste(names(v), format(v), sep = if (is.null(names(v))) "" else "=", collapse = " "), "\n")
}
try1 <- function(expr) show(tryCatch(expr, error = function(e) e))
a <- array(1:24, c(2, 3, 4))
try1(aperm(a))
try1(aperm(a, c(2, 1, 3)))
try1(aperm(a, c(3, 1, 2)))
try1(.Internal(aperm(a, c(2, 3, 1), FALSE)))
# dimnames, their names, and names(dim)
b <- array(1:6, c(2, 3), dimnames = list(r = c("a", "b"), c = c("x", "y", "z")))
try1(aperm(b))
try1(aperm(b, c("c", "r")))
try1(aperm(b, c(2, 1), resize = FALSE))
d <- array(1:6, c(p = 2, q = 3))
try1(aperm(d))
e <- array(1:8, c(2, 2, 2), dimnames = list(c("u", "v"), NULL, c("s", "t")))
try1(aperm(e, c(3, 2, 1)))
# every type
try1(aperm(matrix(c(TRUE, NA, FALSE, TRUE), 2)))
try1(aperm(matrix(c(1.5, 2.5, NA, 4), 2)))
try1(aperm(matrix(c(1+2i, 3i, NA, 4), 2)))
try1(aperm(matrix(c("a", NA, "c", "d"), 2)))
try1(aperm(matrix(as.raw(1:4), 2)))
try1(aperm(matrix(list(1, "a", NULL, 2:3), 2)))
try1(aperm(array(1:3, 3)))
try1(aperm(array(integer(), c(0, 3))))
# a compact sequence with a dim
s <- 1:6
dim(s) <- 2:3
try1(aperm(s))
# perm as double, logical, with NA
try1(aperm(a, c(2.9, 1, 3)))
try1(aperm(a, c(2L, NA, 1L)))
# errors
try1(aperm(1:3))
try1(.Internal(aperm(1:3, NULL, TRUE)))
try1(aperm(a, 1:2))
try1(aperm(a, c(1, 1, 2)))
try1(aperm(a, c(1, 2, 4)))
try1(aperm(a, c(0, 1, 2)))
try1(aperm(b, c("c", "zz")))
try1(aperm(a, c("a", "b", "c")))
try1(aperm(array(1:4, c(2, 2), dimnames = list(c("a", "b"), c("c", "d"))), c("a", "b")))
try1(.Internal(aperm(a, NULL, NA)))
try1(.Internal(aperm(array(expression(1, 2), 2), NULL, TRUE)))
try1(.Internal(aperm(a, NULL)))
f <- function(x) aperm(x, 1)
try1(f(a))
