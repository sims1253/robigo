# lengths() on an object takes its length from a dispatched length():
# the result is allocated as allocVector would, so a bad length errors
length.foo <- function(x) -1L
print(tryCatch(lengths(structure(list(1, 2), class = "foo")), error = conditionMessage))
length.foo <- function(x) NA_integer_
print(tryCatch(lengths(structure(list(1, 2), class = "foo")), error = conditionMessage))
length.foo <- function(x) NaN
print(tryCatch(lengths(structure(list(1, 2), class = "foo")), error = conditionMessage))
length.foo <- function(x) 1L
print(lengths(structure(list(1:3, 2), class = "foo")))
