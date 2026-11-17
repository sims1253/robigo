# log() of a compact sequence (1:n): its values, as a vector's
log(1:3)
log(1:6, 2)
log(8, 1:3)
log(1:6, c(2, 10, 3))
x <- 5:1
log(x, base = 10)
f <- function(v, b) log(v, b)
f(1:4, 2)
f(1:4, 2)
g <- function(n) { s <- 0; for (i in 1:2) s <- s + sum(log(1:n)); s }
g(5)
