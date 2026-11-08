# A complex assignment into a function argument that has not been
# forced yet (`x$a$b <- v` as the argument's first touch) keeps the
# object's attributes: the general applydefine path evaluates the
# symbol (forcing the promise) and hands that value to the setters.
f <- function(x) { x$a$b <- 2L; x }
y <- structure(list(a = list(b = 1L)), class = "foo", other = "keep")
z <- f(y)
print(class(z))
print(attr(z, "other"))
print(z$a$b)
print(class(y))

g <- function(x) { x[["a"]][["b"]] <- 3L; x }
z2 <- g(y)
print(class(z2))
print(z2$a$b)

h <- function(x) { x$a <- list(b = 4L); x }
print(class(h(y)))

# the same at top level, where the binding is already a value
y$a$b <- 5L
print(class(y))
print(y$a$b)

# a failed nested assignment into a default argument leaves the
# binding (and its missing-argument flag) exactly as it was
mf <- function(x = stop("boom")) {
  tryCatch(x$a$b <- 1L, error = function(e) NULL)
  missing(x)
}
print(mf())
