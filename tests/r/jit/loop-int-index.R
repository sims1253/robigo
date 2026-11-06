# Integer indices of vectors and matrices in native loops (jit.rs,
# `Gen::index`): 1..length is read and written in place, anything else
# (0, negative, NA, past the end) leaves native code for the VM.
rd <- function(n, j) {
  x <- c(5L, 6L, 7L); d <- c(0.5, 1.5, 2.5); r <- 0L; q <- 0; jj <- j
  for (i in 1:n) {
    k <- if (i == n) jj else (i %% 3L) + 1L
    r <- x[k]; q <- d[k]
  }
  list(r, q)
}
for (j in list(1L, 3L, 4L, 0L, -1L, NA_integer_, 2147483647L)) { invisible(rd(40, j)); print(rd(40, j)) }
wr <- function(n, j) {
  x <- integer(3); jj <- j
  for (i in 1:n) {
    k <- if (i == n) jj else (i %% 3L) + 1L
    x[k] <- i
  }
  x
}
for (j in list(3L, 4L, 0L, -2L)) { invisible(wr(40, j)); print(wr(40, j)) }
print(tryCatch(wr(40, NA_integer_), error = function(e) conditionMessage(e)))
print(tryCatch(wr(40, NA_integer_), error = function(e) conditionMessage(e)))
w2 <- function(n, j) {
  x <- numeric(3); jj <- j
  r <- tryCatch(for (i in 1:n) {
    k <- if (i == n) jj else (i %% 3L) + 1L
    x[[k]] <- i
  }, error = function(e) conditionMessage(e))
  list(r, x)
}
for (j in list(2L, 4L, 0L, -1L, NA_integer_)) { invisible(w2(40, j)); print(w2(40, j)) }
mt <- function(n, a, b) {
  m <- matrix(1:6, 2); r <- 0L; aa <- a; bb <- b
  for (i in 1:n) {
    p <- if (i == n) aa else 1L
    q <- if (i == n) bb else 2L
    r <- m[p, q]
  }
  r
}
for (ab in list(c(2L, 3L), c(3L, 1L), c(1L, 4L), c(0L, 1L), c(NA, 1L), c(2L, 2L))) {
  invisible(tryCatch(mt(40, ab[1], ab[2]), error = function(e) NULL))
  print(tryCatch(mt(40, ab[1], ab[2]), error = function(e) conditionMessage(e)))
}
