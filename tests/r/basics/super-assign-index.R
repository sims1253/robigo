# x[[i]] <<- v and x[i] <<- v from compiled code: the binding used,
# copies, growth, type changes, lists, names, locks, dispatch, values.
v <- c(1, 2, 3)
f <- function() for (i in 1:3) v[[i]] <<- v[[i]] * 10
f()
print(v)
outer <- function() {
  x <- c(5, 6)
  keep <- x
  g <- function() { x <- "local"; for (i in 1:2) x[[i]] <<- i * 3; x }
  c(g(), x, keep)
}
print(outer())
grow <- function() { y <- numeric(2); h <- function() for (i in 1:4) y[i] <<- i / 4; h(); y }
print(grow())
ty <- function() { z <- 1:3; h <- function() { z[[2]] <<- 0.5; z[3] <<- 7L }; h(); z }
print(ty())
lst <- function() { l <- list(1, "a"); h <- function() { l[[2]] <<- list(9); l[[1]] <<- 3 }; h(); l }
print(lst())
nm <- function() { a <- c(p = 1, q = 2); h <- function() for (i in 1:2) a[[i]] <<- a[[i]] + 100; h(); a }
print(nm())
lk <- function() {
  w <- c(1, 2)
  lockBinding("w", environment())
  h <- function() w[[1]] <<- 5
  tryCatch(h(), error = function(e) conditionMessage(e))
}
print(lk())
gl <- function() for (i in 1:2) newglob[i] <<- i
print(tryCatch(gl(), error = function(e) conditionMessage(e)))
cls <- function() {
  o <- structure(list(1, 2), class = "boxy")
  h <- function() o[[1]] <<- 42
  h()
  o
}
`[[<-.boxy` <- function(x, i, value) { cat("method\n"); x <- unclass(x); x[[i]] <- value * 2; structure(x, class = "boxy") }
print(unclass(cls()))
val <- function() { u <- c(1, 2); h <- function() (u[[2]] <<- 8) + 1; c(h(), u) }
print(val())
oob <- function() { s <- c(1, 2); h <- function() s[[5]] <<- 9; h(); s }
print(oob())
nai <- function() { s <- c(1, 2); h <- function(k) s[[k]] <<- 9; tryCatch(h(NA), error = function(e) conditionMessage(e)) }
print(nai())
