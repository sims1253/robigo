# array.c do_rowscols (row, col, .row, .col) and do_asplit.
show <- function(x) {
  if (inherits(x, "error")) {
    cat("error:", conditionMessage(x), "| call:", deparse(conditionCall(x)), "\n")
    return(invisible())
  }
  elts(x, typeof(x))
  a <- attributes(x)
  for (n in names(a)) elts(a[[n]], paste0("  ", n))
  invisible()
}
# a vector's elements by cat() (no format() needed), a list's one by one
elts <- function(v, label) {
  if (is.list(v)) {
    cat(label, ": list of", length(v), if (!is.null(names(v))) paste("names:", paste(names(v), collapse = ",")), "\n")
    for (i in seq_along(v)) elts(v[[i]], paste0(label, "[[", i, "]]"))
  } else {
    cat(label, ":", if (is.null(v)) "NULL" else v, if (!is.null(names(v))) paste("| names:", paste(names(v), collapse = ",")), "\n")
  }
}
try1 <- function(expr) show(tryCatch(expr, error = function(e) e))
m <- matrix(1:6, 2, dimnames = list(c("a", "b"), c("x", "y", "z")))
try1(row(m))
try1(col(m))
try1(.row(c(3, 2)))
try1(.col(c(3L, 2L)))
try1(row(m, as.factor = TRUE))
try1(col(m, as.factor = TRUE))
try1(row(matrix(numeric(), 0, 3)))
try1(row(1:3))
try1(col(array(1:8, c(2, 2, 2))))
try1(.Internal(row(c(2, NA))))
try1(.Internal(col(c(-1, 2))))
try1(.Internal(row(c("2", "3"))))
try1(.Internal(row(NULL)))
try1(.Internal(col(1, 2)))
# asplit
a <- array(1:12, c(2, 3, 2), dimnames = list(r = c("a", "b"), c = c("x", "y", "z"), k = c("p", "q")))
try1(asplit(m, 1))
try1(asplit(m, 2))
try1(asplit(a, c(1, 3)))
try1(asplit(a, "c"))
try1(asplit(a, 2, drop = TRUE))
try1(asplit(matrix(c(TRUE, FALSE, NA, TRUE), 2), 1))
try1(asplit(matrix(c(1.5, 2, 3, 4), 2), 2))
try1(asplit(matrix(c(1i, 2, 3, 4), 2), 1))
try1(asplit(matrix(letters[1:4], 2), 1))
try1(asplit(matrix(list(1, "a", NULL, 2), 2), 2))
try1(asplit(matrix(as.raw(1:4), 2), 1))
try1(asplit(1:3, 1))
try1(asplit(m, 3))
try1(asplit(m, "nope"))
try1(.Internal(asplit(expression(1), 1L, 1L, NULL, NULL, 1L, 1L, FALSE)))
# asplit's sizes are asInteger'd (with its warnings); a negative one is
# allocVector's error; y is shaped before x's type is looked at
try1(withCallingHandlers(.Internal(asplit(1:4, 2L, 2L, NULL, NULL, 2+1i, 2L, FALSE)),
  warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }))
try1(.Internal(asplit(1:4, 2L, 2L, NULL, NULL, 2L, -1L, FALSE)))
try1(.Internal(asplit(1:4, 2L, 2L, NULL, NULL, 2L, NA, FALSE)))
try1(.Internal(asplit(1:4, 3L, 2L, NULL, NULL, 2L, 2L, FALSE)))
try1(.Internal(asplit(expression(1), 2L, 1L, NULL, NULL, 1L, 1L, FALSE)))
try1(.Internal(asplit(1:4, 0L, 2L, NULL, NULL, -1L, 0L, FALSE)))
