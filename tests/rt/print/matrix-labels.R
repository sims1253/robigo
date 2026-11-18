# printMatrix's labels: dimnames or [i,] / [,j], the names of the dimnames
x <- matrix(1:4, 2, dimnames = list(c("a", "b"), c("x", "y"))); x
dimnames(x) <- list(c("a", "bb"), NULL); x
dimnames(x) <- list(NULL, c("long column", "y")); x
dimnames(x) <- list(R = c("a", "b"), C = c("x", "y")); x
dimnames(x) <- list(Rows = c("a", "b"), NULL); x
dimnames(x) <- list(NULL, Cols = c("x", "y")); x
dimnames(x) <- list(c(r = "a", "b"), c("x", "y"))
names(dimnames(x)) <- c("a much longer row title", "C"); x
names(dimnames(x)) <- c("", ""); x
dimnames(x) <- list(c(NA, "b"), c("x", NA)); x
dimnames(x) <- list(c("", "b"), c("", "y")); x
s <- matrix(c("a", "b", "c", "d"), 2, dimnames = list(c("r1", "r2"), c("c1", "a wide label")))
s
print(s, quote = FALSE)
print(s, right = TRUE)
dimnames(s) <- list(NULL, c("x", NA)); print(s, right = TRUE); print(s)
l <- matrix(c(TRUE, FALSE, NA, TRUE), 2, dimnames = list(c("alpha", "b"), c("y", "zzzzzz")))
l
r <- matrix(as.raw(1:4), 2, dimnames = list(c("a", "b"), c("long name", "y"))); r
z <- matrix(c(1i, 2), 1, dimnames = list("row", c("complex col", "c"))); z
u <- matrix(1:4, 2, dimnames = list(c("été", "b"), c("ü", "y"))); u
big <- matrix(1:20, 10); big
# a "bytes" label cannot be translated: translateChar's error
k <- "caf\xe9"; Encoding(k) <- "bytes"
tryCatch(print(matrix(1, dimnames = list(NULL, k))), error = function(e) print(e))
tryCatch(print(matrix(1, dimnames = structure(list("a", "b"), names = c(k, "C")))), error = function(e) print(e))
tryCatch(print(array(1:2, c(1, 1, 2), dimnames = list(NULL, NULL, c(k, "b")))), error = function(e) print(e))
