# Loops long enough to run as native code must behave exactly like R
sumsq <- function(n) { s <- 0; for (i in 1:n) s <- s + i * i; s }
sumsq(100)
isum <- function(n) { s <- 0L; for (i in 1:n) s <- s + i; s }
isum(1000)
overflow <- function() { s <- .Machine$integer.max - 50L; for (i in 1:100) s <- s + 1L; s }
overflow()
whilena <- function() { x <- 1; i <- 0L; while (x > 0) { i <- i + 1L; if (i == 40L) x <- NA else x <- x + 1 }; i }
tryCatch(whilena(), error = function(e) conditionMessage(e))
ifna <- function() { r <- 0; for (i in 1:50) { v <- if (i == 30) NA else i; if (v > 10) r <- r + 1 }; r }
tryCatch(ifna(), error = function(e) conditionMessage(e))
oob <- function() { x <- c(1.5, 2.5, 3.5); s <- 0; for (i in 1:40) s <- s + x[i %% 4]; s }
oob()
oob2 <- function() { x <- numeric(10); for (i in 1:40) { j <- i %% 12 + 1; x[j] <- i }; x }
oob2()
emptyfor <- function() { k <- 5L; for (j in 1:30) { for (k in seq_len(j - 1)) NULL }; k }
emptyfor()
emptyfor2 <- function() { r <- 0; for (j in 1:30) { k <- -1; for (k in seq_len(0)) r <- r + 1 }; list(r, k) }
emptyfor2()
brk <- function() { s <- 0; for (i in 1:100) { if (i > 50) break; if (i %% 2 == 0) next; s <- s + i }; s }
brk()
retype <- function() { x <- 0; for (i in 1:40) { if (i == 20) x <- 1L else x <- x + 1 }; c(x, typeof(x) == "double") }
retype()
alias <- function() { x <- c(1, 2, 3, 4); y <- x; for (i in 1:40) x[i %% 4 + 1] <- x[i %% 4 + 1] + i; list(x, y) }
alias()
s <- 0
for (i in 1:200) s <- s + i / 2
s
v <- integer(50)
for (i in seq_along(v)) v[i] <- i * 2L
sum(v)
ops <- function() { a <- 0; b <- 0L; c <- 0; for (i in 1:60) { a <- a + (i %% 7) + (i %/% 3); b <- b + (i %% 5L) + (i %/% 4L); c <- c + 2^(i %% 5) }; c(a, b, c) }
ops()
neg <- function() { r <- 0; for (i in 1:30) r <- r + sqrt(i - 20); r }
neg()
andna <- function() { n <- 0L; x <- 1; for (i in 1:40) { x <- if (i == 25) NA else 1; if (i < 30 && x > 0) n <- n + 1L }; n }
tryCatch(andna(), error = function(e) conditionMessage(e))
ornas <- function() { n <- 0L; for (i in 1:40) { a <- i > 20; if (a || i == 5L) n <- n + 1L }; n }
ornas()
nested <- function() {
  m <- 0
  for (a in 1:20) {
    for (b in 1:a) {
      m <- m + a * b
      if (a == 17 && b == 9) m <- m + 0.5
    }
  }
  m
}
nested()
deoptmid <- function() {
  t <- 0L
  for (i in 1:60) {
    t <- t + i
    if (i == 45) t <- t + .Machine$integer.max
  }
  t
}
deoptmid()
lastval <- function() { for (i in 3:47) NULL; i }
lastval()
down <- function() { s <- 0; for (i in 50:1) s <- s * 0.5 + i; s }
print(down(), digits = 17)
mixed <- function() { s <- 0; k <- 0L; for (i in 1:80) { k <- k + 1L; s <- s + k / 3 - i * 0.25 }; c(s, k) }
print(mixed(), digits = 17)
whileint <- function() { i <- 0L; z <- 0; while (i < 100L && z < 1e6) { z <- z * 1.5 + i; i <- i + 1L }; c(i, z) }
print(whileint(), digits = 17)
