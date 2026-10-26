# Vectorised arithmetic chains on long vectors (allocation of temporaries).
set.seed(1)
x <- as.numeric(1:2e6) / 7
y <- rev(x)
s <- 0
for (k in 1:10) {
  z <- (x * 2 + y) / (1 + abs(x - y))
  s <- s + sum(z)
  w <- sqrt(x) * exp(-y / 1e6) + log1p(x)
  s <- s + sum(w)
}
cat(sprintf("%.6f", s), "\n")
