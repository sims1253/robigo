# A replacement that fails before or after it takes its object keeps the
# binding: arity and argument matching, a dispatched as.character method,
# a warning turned into an error, a failed coercion, and the nested
# in-place path (`names(l$a) <- v`).
z <- 1:3
try(attr(z, "a", b = 1) <- 2)
print(z)
try(attributes(z, 2) <- NULL)
print(z)
as.character.zz <- function(x, ...) stop("boom")
try(names(z) <- structure(1:3, class = "zz"))
print(z)
try(length(z) <- Inf)
print(z)
try(storage.mode(z) <- "foo")
print(z)
try(environment(z) <- 1)
print(z)
l <- list(a = 1:3)
try(names(l$a) <- 1:5)
print(l)
# R: coerce.c:R_set_class strips the class in place before coercing.
y <- list(1:2)
try(class(y) <- "double")
print(y)
f <- function() {
  v <- c(a = 1, b = 2)
  try(levels(v) <- function() 1)
  try(levels(v) <- c(1, 2, 1))
  print(v)
  try(dimnames(v) <- list(1:3))
  v
}
print(f())
options(warn = 2)
try(dim(z) <- 3 + 1i)
print(z)
g <- sin
try(attr(g, "foo") <- 1)
print(attributes(g))
