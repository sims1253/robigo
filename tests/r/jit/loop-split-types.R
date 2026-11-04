# A variable holding a double and an integer at different points of a loop
# (awf/Mandelbrot's byteAcc and sum): its value and type wherever the loop
# is left, normally or not.
mb <- function(size) {
  sum <- 0; byteAcc <- 0; bitNum <- 0; y <- 0
  while (y < size) {
    ci <- (2.0 * y / size) - 1.0
    x <- 0
    while (x < size) {
      zr <- 0.0; zrzr <- 0.0; zi <- 0.0; zizi <- 0.0
      cr <- (2.0 * x / size) - 1.5
      z <- 0; notDone <- TRUE; escape <- 0
      while (notDone && (z < 50)) {
        zr <- zrzr - zizi + cr
        zi <- 2.0 * zr * zi + ci
        zrzr <- zr * zr; zizi <- zi * zi
        if ((zrzr + zizi) > 4.0) { notDone <- FALSE; escape <- 1 }
        z <- z + 1
      }
      byteAcc <- bitwShiftL(byteAcc, 1) + escape
      bitNum <- bitNum + 1
      if (bitNum == 8) {
        sum <- bitwXor(sum, byteAcc); byteAcc <- 0; bitNum <- 0
      } else if (x == (size - 1)) {
        byteAcc <- bitwShiftL(byteAcc, 8 - bitNum)
        sum <- bitwXor(sum, byteAcc); byteAcc <- 0; bitNum <- 0
      }
      x <- x + 1
    }
    y <- y + 1
  }
  sum
}
for (k in 1:20) r <- mb(14)
print(r); print(typeof(r)); print(mb(9)); print(mb(30))
# The type a loop leaves a variable with, at its end and where it stops.
t1 <- function(n, stopat) {
  v <- 0; last <- 0
  for (i in 1:n) {
    v <- i
    if (i == stopat) break
    last <- v * 2
    v <- last + 0.5
    if (i == n - 3) v <- NA_integer_
  }
  list(v, last)
}
for (k in 1:20) t1(30, 100)
show <- function(l) print(vapply(l, function(v) paste(typeof(v), format(v)), ""))
show(t1(30, 100)); show(t1(30, 7)); show(t1(5, 2))
t2 <- function(n) {
  a <- 1L; s <- 0
  for (i in 1:n) {
    a <- a * 2L      # integer overflow leaves native code
    s <- s + a
    a <- a / 2 + 1   # a double again
    a <- as.integer(a)
  }
  c(a, s)
}
for (k in 1:20) t2(20)
print(t2(20)); print(t2(40))
t3 <- function(n) {
  x <- TRUE; k <- 0
  for (i in 1:n) {
    x <- !x
    k <- k + x
    x <- (k %% 3) == 0
  }
  list(x, k)
}
for (k in 1:20) t3(10)
show(t3(25))
