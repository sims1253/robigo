library(stats)
for (seed in c(1L, 42L, 12345L)) {
  for (m in c(0, 1, 3, 20, 200)) for (n in c(0, 1, 4, 25)) {
    set.seed(seed)
    cat(seed, m, n, paste(rwilcox(8, m, n), collapse=','), '\n')
  }
  for (n in c(0, 1, 4, 25, 200, 2147483648, Inf)) {
    set.seed(seed)
    cat(seed, n, paste(rsignrank(8, n), collapse=','), '\n')
  }
}
