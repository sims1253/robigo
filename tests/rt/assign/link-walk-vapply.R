# The link-walk family (link-walk.R): vapply's closure. X is a promise in
# its frame, so FUN's `x[2] <<- "Z"` copies and the result is named by the
# old X: a b c. (link-walk.R calls the .Internal, which writes in place.)
reset <- function() {
  x <<- c("a", "b", "c")
  NULL
}
reset()
print(vapply(x, function(e) { x[2] <<- "Z"; 1 }, 0))
print(x)
reset()
print(vapply(x, function(e) { x[2] <<- "Z"; 1 }, 0))
reset()
print(vapply(x, function(e) { x[2] <<- "Z"; 1 }, 0))
