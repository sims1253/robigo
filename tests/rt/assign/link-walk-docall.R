# The link-walk family (link-walk.R): do.call. The call it builds holds the
# list's elements, and the list holds x, so the callee's `x[3] <<- 99`
# copies and `a` is the old vector: 1 2 3. `args[[1]][3] <<- 99` writes the
# list in place but copies its element, which the built call also holds.
reset <- function() {
  x <<- c(1, 2, 3)
  NULL
}
reset()
print(do.call(function(a) { x[3] <<- 99; a }, list(x)))
print(x)
reset()
print(do.call(function(a) { x[3] <<- 99; a }, list(x)))
reset()
print(do.call(function(a) { x[3] <<- 99; a }, list(x)))
args <- list(c(1, 2, 3))
f <- function(a) { args[[1]][3] <<- 99; a }
print(do.call(f, args))
print(args[[1]])
g <- function(...) .Internal(do.call(f, args, globalenv()))
args <- list(c(1, 2, 3))
print(g())
print(args[[1]])
args <- list(c(1, 2, 3))
print(g())
