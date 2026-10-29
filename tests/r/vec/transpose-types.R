# t() over every vector type, dims and dimnames, empty matrices, and the
# not-a-matrix error for expression vectors (R: array.c:do_transpose).
m <- matrix(1:6, 2, dimnames = list(c("a", "b"), c("x", "y", "z"))); print(t(m)); print(t(t(m)))
print(t(matrix(c(1.5, NA, NaN, Inf, -0, 2), 3))); print(t(matrix(c(TRUE, NA, FALSE, TRUE), 1)))
print(t(matrix(c(1i, 2+3i, NA, 4), 2))); print(t(matrix(letters[1:6], 3))); print(t(matrix(as.raw(1:6), 2)))
print(t(matrix(list(1, "a", TRUE, NULL), 2))); print(t(1:5)); print(t(c(a = 1, b = 2)))
print(dim(t(matrix(numeric(0), 0, 3)))); print(dim(t(matrix(numeric(0), 4, 0)))); print(t(matrix(1:12, 4)))
x <- 1:3; dim(x) <- 3; dimnames(x) <- list(c("p", "q", "r")); print(t(x))
print(t(matrix(1:6, 2, dimnames = list(R = c("a", "b"), C = c("x", "y", "z")))))
print(tryCatch(t(as.expression(list(1, quote(a), 2))), error = conditionMessage))
