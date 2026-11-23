# x <<- v in compiled loops: the binding found, copies, types, locks,
# active bindings, the global fallback, and the value of the expression.
counter <- function(n) {
  x <- 0
  f <- function() { while (x < n) x <<- x + 1; x }
  r <- f()
  c(r, x)
}
print(counter(5))
shared <- function() {
  x <- 1
  y <- x
  f <- function() for (i in 1:3) x <<- x + i
  f()
  c(x, y)
}
print(shared())
types <- function() {
  x <- 1L
  f <- function() { for (i in 1:2) x <<- x + 1L; x <<- x + 0.5; x <<- x > 2 }
  f()
  x
}
print(types())
nested <- function() {
  x <- 10
  g <- function() { h <- function() for (i in 1:2) x <<- x * 2; h(); x }
  c(g(), x)
}
print(nested())
glob <- function() for (i in 1:3) gx <<- i
glob()
print(gx)
lockd <- function() {
  x <- 1
  e <- environment()
  lockBinding("x", e)
  f <- function() x <<- x + 1
  tryCatch(f(), error = function(err) conditionMessage(err))
}
print(lockd())
act <- function() {
  e <- environment()
  store <- 0
  makeActiveBinding("x", function(v) if (missing(v)) store else store <<- v * 10, e)
  f <- function() for (i in 1:2) x <<- i
  f()
  store
}
print(act())
vis <- function() { x <- 0; f <- function() x <<- 5; f() }
vis()
print(vis())
val <- function() { x <- 0; f <- function() (x <<- 7) + 1; c(f(), x) }
print(val())
fn <- function() {
  g <- function(a) a + 1
  f <- function() { for (i in 1:2) g <<- function(a) a * 100; g(2) }
  c(f(), g(2))
}
print(fn())
