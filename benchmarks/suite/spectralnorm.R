# spectral norm (Benchmarks Game), scalar loops with a small function.
a <- function(i, j) 1 / ((i + j) * (i + j + 1) / 2 + i + 1)
av <- function(v, n) { out <- numeric(n); for (i in 0:(n - 1)) { s <- 0; for (j in 0:(n - 1)) s <- s + a(i, j) * v[j + 1]; out[i + 1] <- s }; out }
atv <- function(v, n) { out <- numeric(n); for (i in 0:(n - 1)) { s <- 0; for (j in 0:(n - 1)) s <- s + a(j, i) * v[j + 1]; out[i + 1] <- s }; out }
spectral <- function(n) {
  u <- rep(1, n)
  for (k in 1:10) { v <- atv(av(u, n), n); u <- atv(av(v, n), n) }
  sqrt(sum(u * v) / sum(v * v))
}
cat(sprintf("%.9f", spectral(100)), "\n")
