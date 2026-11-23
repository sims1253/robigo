# a:b ranges in compiled code: subsetting and assignment fast paths and
# their fallbacks
f <- function() {
  x <- c(10, 20, 30, 40, 50)
  print(x[2:4]); print(x[4:2]); print(x[5:5]); print(x[0:2]); print(x[4:7])
  i <- 3L
  print(x[i:1]); print(x[(i + 1):2])
  y <- x
  y[1:3] <- y[3:1]; print(y); print(x)
  y[2:3] <- 0; print(y)
  y[4:5] <- c(7L, 8L); print(y); print(typeof(y))
  z <- 1:6
  z[2:3] <- c(1.5, 2.5); print(z); print(typeof(z))
  z <- 1:6
  z[5:8] <- 9L; print(z)
  z <- 1:6
  z[1:4] <- 1:2; print(z)
  z[1:3] <- 1:2; print(z)
  w <- c(a = 1, b = 2, c = 3)
  print(w[1:2]); w[2:3] <- c(9, 8); print(w)
  s <- c("a", NA, "c", "d")
  s[3:4] <- s[1:2]; print(s); print(s[2:3])
  l <- c(TRUE, FALSE, NA)
  l[1:2] <- NA; print(l)
  m <- matrix(1:6, 2)
  m[1:3] <- 0L; print(m); print(m[2:5])
  fct <- factor(c("u", "v", "w"))
  print(fct[2:3])
  lst <- list(1, "a", TRUE)
  print(lst[2:3])
  v <- c(1, 2, 3)
  v[1:3] <- v; print(v)
  v[-1:-2] <- 0; print(v)
  d <- 2.5
  print((1:5)[1:d]); print(x[1.5:3])
  n <- 0L
  print(x[1:n])
  r <- 2:4; print(r); print(typeof(r))
  for (k in (1:3)) cat(k, ""); cat("\n")
  print(sum(1:10)); print(rev(3:1))
  print(x[NA_integer_:2])
}
f()
