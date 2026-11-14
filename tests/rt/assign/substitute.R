# substitute of an argument and of a default (materialized as a promise)
f <- function(x) substitute(x)
f(5)
d <- function(y = 2.5) substitute(y)
d()
h <- function(z) {
  z <- 3L
  substitute(z)
}
h(1)
