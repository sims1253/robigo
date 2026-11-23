# lapply/vapply/sapply with pure functions (run without calls when possible)
f <- function(x) if (x > 2) x * 2 else -x
unlist(lapply(1:5, f))
vapply(c(a = 1.5, b = NA, c = 3), function(v) v + 1, numeric(1))
sapply(list(1L, 2.5, TRUE), function(z) z * 3)
vapply(1:4, function(v) v > 2, logical(1))
lapply(c(x = 1, y = 2), function(v) v / 0)
sapply(1:3, function(v) sqrt(v - 2))
vapply(1:3, function(i) i + .Machine$integer.max - 2L, integer(1))
lapply(1:3, function(k) if (k == 2) NA else k)
tryCatch(vapply(c(1, NA, 3), function(v) if (v > 1) 1 else 0, numeric(1)), error = function(e) conditionMessage(e))
sapply(c("a", "b"), function(s) s)
g <- function(x, ...) x + 1
sapply(1:3, g, 10)
