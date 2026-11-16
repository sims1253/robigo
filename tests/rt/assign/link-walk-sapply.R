# The link-walk family (link-walk.R): sapply calls lapply on its X, a
# promise whose value holds a link, and names the result by that X: FUN's
# `x[2] <<- "Z"` copies, the names stay a b c.
reset <- function() {
  x <<- c("a", "b", "c")
  NULL
}
reset()
print(sapply(x, function(e) { x[2] <<- "Z"; 1 }))
print(x)
reset()
print(sapply(x, function(e) { x[2] <<- "Z"; 1 }))
reset()
print(sapply(x, function(e) { x[2] <<- "Z"; 1 }))
