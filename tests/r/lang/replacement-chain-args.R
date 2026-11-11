# R: eval.c:applydefine evaluates the setter's own arguments with the
# target still bound: R code they run sees it, and an error there leaves
# it as it was.
f <- function() {
  a <- list(b = structure(1, class = "foo"))
  attr(a$b, { print(a); "hello" }) <- 42
  a
}
print(f())
f <- function() {
  a <- list(b = structure(1, class = "foo"))
  try(class(a$b, stop("x")) <- "bar", silent = TRUE)
  print(a)
}
f()
f <- function() {
  a <- list(b = 1, c = list(d = 2L))
  try(attr(a$b, stop("x")) <- "bar", silent = TRUE)
  try(attr(a$c[["d"]], stop("y")) <- "bar", silent = TRUE)
  try(a$c$d[stop("z")] <- 3L, silent = TRUE)
  print(a)
}
f()
# A classed element whose setter takes no other argument is still
# modified where it is.
f <- function() {
  a <- list(b = structure(1, class = "foo"))
  class(a$b) <- "bar"; oldClass(a$b) <- "baz"
  print(a)
}
f()
# R: subassign.c:do_subassign3 checks its arity before it reads the
# object: a failed `$<-` leaves the target bound as it was.
x <- quote(f())
try(`$`(x, a, b) <- 5, silent = TRUE)
print(x)
g <- function() {
  x <- quote(f(1)); y <- x
  try(`$`(x, a, b) <- 5, silent = TRUE)
  print(x); print(identical(x, y))
}
g()
l <- list(a = 1)
try(`$`(l, a, b) <- 5, silent = TRUE)
print(l)
