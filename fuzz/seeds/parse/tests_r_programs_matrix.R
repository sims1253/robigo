a <- matrix(c(2, 1, 1, 3), 2)
b <- matrix(1:4, 2)
a %*% b
a * b
solve(a)
det(a)
diag(2)
crossprod(b)
m <- matrix(1:12, nrow = 3, dimnames = list(c("r1", "r2", "r3"), c("A", "B", "C", "D")))
m
m["r2", "C"]
rowSums(m)
colMeans(m)
nrow(m)
ncol(m)
rbind(m, total = colSums(m))
