# for (i in seq(length = n)) in native loops (convolution_slow), and loops
# over 1:n started by the interpreter.
conv <- function(x, y) {
  nx <- length(x); ny <- length(y)
  z <- numeric(nx + ny - 1)
  for (i in seq(length = nx)) {
    xi <- x[[i]]
    for (j in seq(length = ny)) {
      ij <- i + j - 1
      z[[ij]] <- z[[ij]] + xi * y[[j]]
    }
  }
  z
}
for (k in 1:10) r <- conv(as.double(1:40), as.double(40:1))
print(sum(r)); print(r[c(1, 20, 79)])
print(conv(c(1, 2), c(3, 4, 5)))
print(conv(numeric(0), c(1, 2)))
n0 <- function(m) { s <- 0; for (i in seq(length = m)) s <- s + i; s }
for (k in 1:30) n0(20)
print(c(n0(2.5), n0(0), n0(3L), n0(TRUE)))
print(tryCatch(n0(-1), error = function(e) conditionMessage(e)))
d <- function(m) { s <- 0; for (i in 1:30) for (j in seq(length = m)) s <- s + j; s }
for (k in 1:5) d(4)
print(d(4)); print(d(0)); print(d(4.2))
# A user seq.integer method makes seq() dispatch differently: the call
# runs as written.
seq.integer <- function(...) { cat("seq.integer\n"); 1:2 }
print(n0(5L))
rm(seq.integer)
print(n0(5L))
# Loops at top level (the interpreter's) over a:b, entered natively.
s <- 0
for (i in 1:50) for (j in 1:i) s <- s + j
print(s)
s <- 0
for (i in 50:1) { for (j in 1:3) s <- s + i * j }
print(s)
