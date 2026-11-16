# The link-walk family (link-walk.R): mapply and Map. mapply's
# `dots <- list(...)` holds x, so FUN's `x[2] <<- "Z"` copies: the values
# and the names (from dots[[1L]]) are the old x's.
reset <- function() {
  x <<- c("a", "b", "c")
  NULL
}
reset()
print(mapply(function(e, i) { x[2] <<- "Z"; e }, x, 1:3))
print(x)
reset()
print(mapply(function(e, i) { x[2] <<- "Z"; e }, x, 1:3))
reset()
print(unlist(Map(function(e) { x[2] <<- "Z"; e }, x)))
print(x)
reset()
print(unlist(Map(function(e) { x[2] <<- "Z"; e }, x)))
