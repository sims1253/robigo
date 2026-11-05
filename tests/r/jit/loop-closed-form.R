# Loops that only count and copy run in closed form (jit/summary.rs): their
# final values must be those of running every iteration, including where
# the closed form declines (NA, non-integer starts, overflow, empty `for`).
# Each function is called often enough for native loops, and the `drive`
# functions often enough for closures compiled whole.
show <- function(...) { x <- list(...); for (nm in names(x)) cat(nm, "=", deparse(x[[nm]]), " "); cat("\n") }
w1 <- function(i0, n) { x <- 0; i <- i0; while (i < n) { x <- i; i <- i + 1 }; show(x = x, i = i) }
w2 <- function(i0, n) { i <- i0; while (i <= n) i <- i + 2; show(i = i) }
w3 <- function(i0, n) { i <- i0; while (i > n) i <- i - 3; show(i = i) }
w4 <- function(i0, n) { i <- i0; while (i >= n) { y <- i * 2; i <- i - 1 }; show(i = i, y = if (exists("y", inherits = FALSE)) y else "unset") }
w5 <- function(i0, n) { i <- i0; while (n > i) { z <- i / 3; i <- 1 + i }; show(i = i, z = if (exists("z", inherits = FALSE)) z else "unset") }
w6 <- function(i0, n) { i <- i0; while (i != n) i <- i + 1L; show(i = i) }
w7 <- function(i0, n) { i <- i0; j <- 10; while (i < n) { j <- j - 1; k <- i + j; i <- i + 1 }; show(i = i, j = j, k = if (exists("k", inherits = FALSE)) k else "unset") }
w8 <- function(i0, n) { i <- i0; while (i < n - 1) { b <- i < 5; i <- i + 1 }; show(i = i, b = if (exists("b", inherits = FALSE)) b else "unset") }
f1 <- function(n) { x <- 0; for (i in 1:n) x <- i; show(x = x, i = i) }
f2 <- function(a, b) { for (i in a:b) { }; show(i = i) }
f3 <- function(n) { s <- 0; for (i in seq_len(n)) { s <- s + 1; t <- i * 2.5 }; show(s = s, i = i, t = if (exists("t", inherits = FALSE)) t else "unset") }
f4 <- function(n) { s <- 0L; for (i in 1:n) s <- s + 2L; show(s = s, i = i) }
f5 <- function(n, k) { for (i in 1:n) { y <- i ^ k; x <- k }; show(y = y, x = x) }
cases <- list(
  quote(w1(0, 100)), quote(w1(0, 100.5)), quote(w1(-0, 1)), quote(w1(5, 3)), quote(w1(0L, 50L)),
  quote(w1(2^53 - 40, 2^53)), quote(w1(-50.5, 3)),
  quote(w2(1, 100)), quote(w2(1, 100.9)), quote(w2(1L, 99L)), quote(w2(1, -5)),
  quote(w3(100, 1)), quote(w3(100, 1.5)), quote(w3(100L, -20L)),
  quote(w4(50, 3)), quote(w4(50, 3.2)), quote(w4(1, 3)),
  quote(w5(0, 77)), quote(w5(0L, 77.7)), quote(w5(10, 5)),
  quote(w6(1L, 60L)), quote(w6(1L, 60)),
  quote(w7(0, 40)), quote(w7(0L, 40L)),
  quote(w8(0, 30)), quote(w8(0, 30.5)),
  quote(f1(100)), quote(f1(100L)), quote(f1(1)), quote(f1(-30)),
  quote(f2(10, 100)), quote(f2(100, 10)), quote(f2(-5, 40)), quote(f2(3L, 3L)),
  quote(f3(0)), quote(f3(30)), quote(f3(100)),
  quote(f4(100)),
  quote(f5(50, 2)), quote(f5(50, 0.5)), quote(f5(20, NA))
)
for (cs in cases) {
  cat(deparse(cs), ": ")
  sink("/dev/null"); for (r in 1:3) eval(cs); sink()
  eval(cs)
}
# Integer overflow in an induction variable: the warning and NA.
g <- function(n) { s <- 2147483600L; for (i in 1:n) s <- s + 1L; s }
for (r in 1:3) print(g(100))

# Closures compiled whole.
v1 <- function(i0, n) { x <- 0; i <- i0; while (i < n) { x <- i; i <- i + 1 }; c(x, i) }
v4 <- function(i0, n) { y <- -1; i <- i0; while (i >= n) { y <- i * 2; i <- i - 1 }; c(i, y) }
v7 <- function(i0, n) { k <- -1; i <- i0; j <- 10; while (i < n) { j <- j - 1; k <- i + j; i <- i + 1 }; c(i, j, k) }
e1 <- function(n) { x <- 0L; for (i in 1:n) x <- i; c(x, i) }
e3 <- function(n) { t <- -1; s <- 0; for (i in seq_len(n)) { s <- s + 1; t <- i * 2.5 }; c(s, t) }
e4 <- function(n) { s <- 0L; for (i in 1:n) s <- s + 2L; c(s, i) }
# Locals nothing reads, whose type changes.
d1 <- function(n) { x <- 0; y <- 1L; for (i in 1:n) { x <- i; y <- i * 2.5 }; i }
d2 <- function(a) { z <- a; z <- TRUE; a + 1 }
d3 <- function(n) { x <- 1; if (n > 5) x <- n > 7; n * 2 }
d4 <- function(n) { x <- 0; for (i in 1:n) { x <- i; if (i > 100000L) stop("big") }; i }
d5 <- function(n) { s <- 0; x <- 0L; for (i in 1:n) { x <- s; s <- s + i }; s }
drive <- function(f, a, b) { r <- NULL; for (j in 1:700) r <- f(a, b); r }
drive1 <- function(f, a) { r <- NULL; for (j in 1:700) r <- f(a); r }
print(drive(v1, 0, 100)); print(drive(v1, 0, 100.5)); print(1 / drive(v1, -0, 1)); print(drive(v1, 5, 3))
print(drive(v1, -50.5, 3)); print(drive(v1, 0L, 50L))
print(drive(v4, 50, 3.2)); print(drive(v4, 1, 3))
print(drive(v7, 0, 40))
print(drive1(e1, 100)); print(drive1(e1, -30)); print(drive1(e3, 30)); print(drive1(e4, 100))
print(drive1(d1, 50)); print(drive1(d2, 3)); print(drive1(d3, 6)); print(drive1(d3, 9))
print(drive1(d4, 20)); print(drive1(d5, 40))
print(tryCatch(drive1(d4, 100001L), error = function(e) conditionMessage(e)))
print(drive1(e3, 0))
# An overwritten value that overflows still warns: the loop is not summarized.
o1 <- function(n) { x <- 0L; for (i in 1:n) { x <- i + 2147483600L; x <- i }; x }
for (r in 1:3) print(o1(100))
o2 <- function(n) { i <- 0L; y <- 0L; while (i < n) { y <- i * 100000000L; y <- i; i <- i + 1L }; c(i, y) }
for (r in 1:3) print(o2(30L))
o3 <- function(n) { x <- 0L; for (i in 1:n) { x <- i + 2147483600L; x <- i }; i }
print(drive1(o3, 60))
# `for` loops in closed form in closures compiled whole: induction
# variables at the edges.
e5 <- function(n, s0) { y <- 0; s <- s0; for (i in 1:n) { y <- s; s <- s + 1 }; c(1 / y, s) }
print(drive(e5, 1, -0)); print(drive(e5, 5, -0)); print(drive(e5, 3, 2^53 - 1)); print(drive(e5, 3, 0.5))
e6 <- function(n) { s <- 2147483600L; y <- 0L; for (i in 1:n) { y <- s; s <- s + 1L }; c(y, s) }
print(drive1(e6, 40)); print(drive1(e6, 100))
e7 <- function(n) { s <- 0; d <- 0; for (i in 10:n) { s <- s - 2; d <- i / 4 + s }; c(s, d, i) }
print(drive1(e7, 3)); print(drive1(e7, 10)); print(drive1(e7, 25))
w1(0, NA)
