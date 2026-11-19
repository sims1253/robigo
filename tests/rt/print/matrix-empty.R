# empty matrices and the max.print cut
matrix(numeric(0), 0, 0)
matrix(numeric(0), 0, 3)
matrix(character(0), 2, 0)
matrix(integer(0), 0, 2, dimnames = list(NULL, c("a", "b")))
matrix(logical(0), 3, 0, dimnames = list(c("x", "y", "z"), NULL))
m <- matrix(1:6, 2, dimnames = list(R = c("a", "b"), C = c("x", "y", "z")))
m[0, , drop = FALSE]
m[, 0, drop = FALSE]
print(matrix(1:20, 4), max = 8)
print(matrix(1:20, 4), max = 4)
print(matrix(1:20, 4), max = 2)
print(matrix(1:20, 4), max = 1)
print(matrix(1:3, 1), max = 2)
print(matrix(1:20, 4), max = 0)
options(max.print = 6)
matrix(1:20, 4)
matrix(c("a", "b", "c", "d"), 1)
options(max.print = 99999)
