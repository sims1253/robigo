# The link-walk family (link-walk.R): Reduce and Filter keep x in a promise
# of their frame, so f's `x[3] <<- 99` copies and they go on with the old
# vector: 1 + 2 + 3, not 1 + 2 + 99.
reset <- function() {
  x <<- c(1, 2, 3)
  NULL
}
reset()
print(Reduce(function(a, b) { x[3] <<- 99; a + b }, x))
print(x)
reset()
print(Reduce(function(a, b) { x[3] <<- 99; a + b }, x))
reset()
print(Reduce(function(a, b) { x[3] <<- 99; a + b }, x, accumulate = TRUE))
reset()
print(Filter(function(e) { x[3] <<- 99; e > 1 }, x))
