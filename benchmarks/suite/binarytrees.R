# binary-trees (Benchmarks Game): recursion and list allocation.
make <- function(d) if (d == 0) list(NULL, NULL) else list(make(d - 1), make(d - 1))
check <- function(t) if (is.null(t[[1]])) 1 else 1 + check(t[[1]]) + check(t[[2]])
bt <- function(n) {
  total <- 0
  for (d in seq(4, n, by = 2)) {
    iters <- 2^(n - d + 4)
    s <- 0
    for (i in 1:iters) s <- s + check(make(d))
    total <- total + s
  }
  total
}
cat(bt(10), "\n")
