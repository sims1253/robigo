## as.numeric/as.double, is.matrix/is.array and as.call are internal
## generics in GNU R (coerce.c:do_asatomic, do_is, do_ascall): they
## dispatch on a classed argument, and as.numeric/as.double share the
## generic name "as.double" (names.c gives both PRIMVAL 2).
as.double.foo <- function(x) "as.double.foo"
as.numeric.foo <- function(x) "as.numeric.foo"
is.matrix.foo <- function(x) "is.matrix.foo"
is.array.foo <- function(x) "is.array.foo"
as.call.foo <- function(x) "as.call.foo"
x <- structure(1, class = "foo")
print(as.numeric(x))
print(as.double(x))
print(is.matrix(x))
print(is.array(x))
print(as.call(x))

## No method for the class: the internal code runs.
y <- structure(1:4, dim = c(2L, 2L), class = "bar")
print(as.numeric(y))
print(as.double(y))
print(is.matrix(y))
print(is.array(y))

## Only as.numeric has a method in this class: as.numeric still dispatches
## "as.double" and therefore does not use it.
z <- structure(1, class = "baz")
as.numeric.baz <- function(x) "as.numeric.baz"
print(as.numeric(z))
print(as.double(z))
