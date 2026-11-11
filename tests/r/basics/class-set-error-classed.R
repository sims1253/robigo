# R: coerce.c:R_set_class strips the class before it coerces or checks
# the type. eval.c:EnsureLocal has rebound the target to a duplicate when
# it was shared, so on error the target is left without its class and
# the other binding keeps it.
x <- structure(1, class = "foo"); y <- x
try(class(x) <- "externalptr")
print(attributes(x)); print(y)
x <- structure(1, class = "foo")
try(class(x) <- "name")
print(attributes(x))
f <- function() {
  x <- structure(1, class = "foo"); y <- x
  try(class(x) <- "environment")
  print(attributes(x)); print(y)
}
f()
g <- function(x) { try(class(x) <- "externalptr"); attributes(x) }
z <- structure(1, class = "foo"); print(g(z)); print(z)
x <- structure(list(1), class = "foo", names = "a"); y <- x
class(x) <- "numeric"
print(x); print(y)
# Errors before the strip leave the class.
x <- structure(1, class = "foo")
try(class(x) <- "matrix"); print(x)
try(oldClass(x) <- quote(a)); print(x)
x <- structure(1:4, class = "foo", names = letters[1:4]); y <- x
try(attributes(x) <- list(dim = 2:3)); print(x)
# A classed list element only its container holds is stripped in place
# (evalseq duplicates it only when it may be shared).
h <- function() {
  a <- list(x = structure(1, class = "foo"))
  try(class(a$x) <- "externalptr")
  print(a)
  a <- list(p = list(x = structure(1, class = "foo")))
  try(class(a$p[["x"]]) <- "char")
  print(a)
  a <- list(x = structure(1, class = "foo")); b <- a
  try(class(a$x) <- "externalptr")
  print(a); print(b)
  e <- structure(1, class = "foo"); a <- list(x = e)
  try(class(a$x) <- "externalptr")
  print(a); print(e)
  a <- list(x = structure(1, class = "foo"))
  attr(a$x, "u") <- 2; class(a$x) <- "bar"
  a
}
print(h())
# The non-dispatching setters on objects.
x <- structure(1:3, class = "foo")
attr(x, "names") <- c("a", "b", "c"); oldClass(x) <- "bar"; print(x)
attributes(x) <- list(class = "baz", names = c("p", "q", "r")); print(x)
x <- structure(list(1L), class = "foo")
storage.mode(x) <- "double"; print(x)
k <- structure(function() 1, class = "foo")
environment(k) <- globalenv(); print(class(k))
