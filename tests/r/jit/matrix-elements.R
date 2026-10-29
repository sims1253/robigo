# Cross the native threshold with both index spellings and storage types.
f <- function(x) {
  original <- x
  for (j in 1:3) for (i in 1:40) x[[i, j]] <- i + j
  total <- 0
  for (j in 1:3) for (i in 1:40) total <- total + x[[i, j]]
  for (i in 1:40) x[i, 2] <- x[i, 1] + 1L
  print(c(total, x[40, 2], original[40, 2]))
  print(dim(x))
  x
}
x <- f(matrix(0, 40, 3))
y <- f(matrix(0L, 40, 3))
print(identical(x, y))
# Integer matrices promote through the VM, then resume with a double view.
g <- function() {
  x <- matrix(0L, 40, 2)
  for (i in 1:40) x[[i, 2]] <- if (i < 25L) 2L else 2.5
  print(c(typeof(x), x[24, 2], x[25, 2], x[40, 2]))
}
g()
# Fractional indices truncate; each dimension needs a separate bounds check.
h <- function(k) {
  x <- matrix(0, 40, 2)
  for (i in 1:40) {
    x[[i, 1]] <- i
    x[[i, if (i < 30L) 2 else k]] <- 7
  }
  print(c(sum(x), x[40, 1], x[40, 2]))
}
h(2.9)
for (k in c(3, 0, -1, NA_real_, NaN, Inf))
  tryCatch(h(k), error = function(e) print(conditionMessage(e)))
# Deoptimize a read after earlier writes, preserving both slots and registers.
r <- function(k) {
  x <- matrix(1:80, 40, 2)
  total <- 0L
  for (i in 1:40) {
    x[i, 1] <- i + 1L
    total <- total + x[[i, if (i < 30L) 2 else k]]
  }
  total
}
print(r(2))
tryCatch(r(3), error = function(e) print(conditionMessage(e)))
# Shared data retains the old values while the matrix changes.
a <- matrix(0, 40, 2)
b <- a
for (i in 1:40) a[[i, 2]] <- b[[i, 1]] + i
print(c(sum(a), sum(b)))
# One-element matrices still have matrix types; empty dimensions never access storage.
single <- function() {
  x <- matrix(0, 1, 1)
  for (i in 1:40) x[[1, 1]] <- x[[1, 1]] + 1
  x
}
print(single())
fallback <- function(x) {
  for (i in 1:40) x[[1, 1]] <- i
  x
}
print(fallback(matrix(0, 1, 1, dimnames = list("r", "c"))))
tryCatch(fallback(matrix(numeric(), 0, 2)), error = function(e) print(conditionMessage(e)))
# Native execution mutates rows 17..40 before the row-41 exit to the VM.
bounds <- function(two) {
  x <- matrix(0, 40, 2)
  on.exit(print(c(sum(x), x[40, 2])))
  if (two) {
    for (i in 1:41) x[[i, 2]] <- i
  } else {
    for (i in 1:41) x[i, 2] <- i
  }
}
tryCatch(bounds(TRUE), error = function(e) print(conditionMessage(e)))
tryCatch(bounds(FALSE), error = function(e) print(conditionMessage(e)))
readbounds <- function() {
  x <- matrix(1:80, 40, 2)
  total <- 0L
  on.exit(print(total))
  for (i in 1:41) total <- total + x[[i, 2]]
}
tryCatch(readbounds(), error = function(e) print(conditionMessage(e)))
