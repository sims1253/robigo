# Indices that are unforced promises (arguments) in x[i], x[[i]],
# x[i, j], x[[i, j]]: evaluated after x, only when x is not an object,
# and passed on as missing when they are missing.
v <- c(10, 20, 30)
m <- matrix(1:6, 2)
f1 <- function(i) v[i]
f2 <- function(i) v[[i]]
g1 <- function(i, j) m[i, j]
g2 <- function(i, j) m[[i, j]]
for (k in 1:3) print(c(f1(k), f2(k)))
for (a in 1:2) for (b in 1:3) print(c(g1(a, b), g2(a, b)))
print(f1(k + 0))
print(g2(1, b - 1))
# x is read before the index, whose promise reassigns it
print(f1({ v <- c(-1, -2, -3); 2 }))
print(v)
v <- c(10, 20, 30)
print(f2({ v <<- c(-1, -2, -3); 3 }))
print(v)
print(g2({ m <<- matrix(0L, 2, 3); 2 }, 2))
print(m)
m <- matrix(1:6, 2)
print(g1({ cat("i\n"); 1 }, { cat("j\n"); 3 }))
# missing indices
print(f1())
print(g1(, 2))
print(g1(2, ))
h <- function(i) f1(i)
print(h())
tryCatch(print(f2()), error = function(e) print(conditionMessage(e)))
tryCatch(print(g2(1, )), error = function(e) print(conditionMessage(e)))
# out of range / errors from the promise
tryCatch(print(f2(4)), error = function(e) print(conditionMessage(e)))
tryCatch(print(g2(3, 1)), error = function(e) print(conditionMessage(e)))
tryCatch(print(f1(stop("boom"))), error = function(e) print(conditionMessage(e)))
tryCatch(print(g2(1, stop("bam"))), error = function(e) print(conditionMessage(e)))
# objects dispatch with the promise unforced
o <- structure(list(1, 2), class = "obj")
`[.obj` <- function(x, i) if (missing(i)) "missing" else "dispatched"
`[[.obj` <- function(x, i, j) substitute(i)
fo1 <- function(i) o[i]
fo2 <- function(i, j) o[[i, j]]
print(fo1())
print(fo1(stop("never")))
print(fo2(a + b, 1))
for (k in 1:2) print(f1(k) + g2(k, k))
# defaults and default-valued formals
d <- function(i = 2) v[i]
print(d())
d2 <- function(i = 1, j = 3) m[[i, j]]
print(d2())
print(d2(2))
