# Matrix operations: products, transposes, apply, colSums, solve.
set.seed(5)
m <- matrix(sin(1:250000), 500)
p <- m %*% t(m)
cs <- colSums(m) + rowMeans(m)
a <- apply(m[1:100, ], 1, function(r) sum(r^2))
s <- solve(p[1:100, 1:100] + diag(100), rep(1, 100))
cat(sprintf("%.6f", c(sum(p), sum(cs), sum(a), sum(s))), "\n")
