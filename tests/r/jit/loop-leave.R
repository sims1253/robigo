# Loops that run natively except at calls native code does not have (cat,
# a closure that is not pure): it leaves there, the VM runs them, and the
# loop runs natively again from its next iteration.
f <- function(n) {
  s <- 0; k <- 0L
  for (i in 1:n) {
    s <- s + i * 0.5
    if (i %% 7L == 0) {
      cat("at", i, "s =", s, "\n")
      k <- k + 1L
    }
  }
  c(s, k)
}
for (j in 1:3) print(f(30))
side <- 0
g <- function(i) { side <<- side + i; side }
h <- function(n) {
  x <- 0
  for (i in 1:n) {
    x <- x + 1
    if (i %% 4 == 0) x <- x + g(i)
  }
  x
}
for (j in 1:5) print(h(20)); print(side)
# The call changes a variable's type: the loop continues in the VM.
ty <- function(n) {
  v <- 0
  for (i in 1:n) {
    v <- v + 1
    if (i == 10) v <- as.character(v)
    if (is.character(v)) break
  }
  v
}
for (j in 1:3) print(ty(20))
# Every iteration calls: native code is not used for it.
ev <- function(n) { s <- 0; for (i in 1:n) s <- s + nchar(format(i)); s }
print(ev(50))
# Inside a loop of loops.
nest <- function(n) {
  out <- 0
  for (i in 1:n) {
    for (j in 1:n) {
      out <- out + i * j
      if (j == n && i %% 3 == 0) cat("row", i, out, "\n")
    }
  }
  out
}
for (j in 1:3) print(nest(20))
# as.integer, as.double and if() on integers and doubles in native loops.
cv <- function(xs) {
  s <- 0L; d <- 0
  for (i in 1:length(xs)) {
    x <- xs[i]
    s <- s + as.integer(x > 1)
    d <- d + as.double(s)
    if (s) d <- d + 0.5
    if (x - 2.5) d <- d + 1
    s <- s + as.integer(x)
  }
  c(s, d)
}
xs <- rep(c(1, 2, 3, 2.5, 7.9), 5)
for (j in 1:10) cv(xs)
print(cv(xs))
print(tryCatch(cv(c(xs, 3e9, 4)), warning = function(w) conditionMessage(w)))
print(tryCatch(suppressWarnings(cv(c(xs, 3e9, 4))), error = function(e) conditionMessage(e)))
print(tryCatch(cv(c(xs, NA, 3)), error = function(e) conditionMessage(e)))
print(tryCatch(cv(c(xs, NaN, 3)), error = function(e) conditionMessage(e)))
