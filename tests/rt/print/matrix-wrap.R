# printMatrix's columns wrapped to the width
matrix(1:60, 2)
matrix(rnorm <- seq(0.001, 2, length.out = 30), 2)
options(width = 40)
matrix(1:30, 2)
matrix(c("aaaaaaaaaa", "b", "cccccccc", "d", "eeeeeeeeeeee", "f", "g", "hhhhhhh"), 1)
print(matrix(c("aaaaaaaaaa", "b", "cccccccc", "d", "eeeeeeeeeeee", "f", "g", "hhhhhhh"), 1), right = TRUE)
x <- matrix(1:24, 2, dimnames = list(c("first row", "second"), paste0("column", 1:12)))
x
options(width = 80)
print(matrix(1:30, 2), width = 30)
print(matrix(1:2, 1), width = 10)
