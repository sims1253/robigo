# Comparisons that decide the next jump branch directly in native code
# (jit.rs, Gen::compare_branch): NA conditions with doubles and integers,
# if and while, negations, and the closure tier.
f1 <- function(x, n) { s <- 0; i <- 0; while (i < n) { if (x[[i + 1]] > 2) s <- s + 1 else s <- s - 1; i <- i + 1 }; s }
x <- c(1, 3, 5, 2, 7, 0.5, 9, 4)
for (r in 1:20) a <- f1(x, 8); print(a)
xn <- x; xn[6] <- NA
for (r in 1:3) print(tryCatch(f1(xn, 8), error = function(e) conditionMessage(e)))
f2 <- function(n, lim) { k <- 0L; for (i in 1:n) { if (i >= lim) k <- k + 1L }; k }
for (r in 1:20) a <- f2(100L, 37L); print(a)
for (r in 1:3) print(tryCatch(f2(100L, NA_integer_), error = function(e) conditionMessage(e)))
f3 <- function(n, lim) { k <- 0; j <- 0; while (j != lim) { j <- j + 1; if (j == 5 || j <= 2) k <- k + 10 }; c(j, k) }
for (r in 1:20) a <- f3(100, 30); print(a)
f4 <- function(v) { m <- -Inf; for (i in seq_along(v)) if (!(v[[i]] <= m)) m <- v[[i]]; m }
for (r in 1:20) a <- f4(c(3, 8, 1, 9, 2)); print(a)
for (r in 1:3) print(tryCatch(f4(c(3, NaN, 1)), error = function(e) conditionMessage(e)))
drive <- function() { r <- 0; for (j in 1:700) r <- f2(60L, 7L) + f3(0, 12)[2]; r }
print(drive())
