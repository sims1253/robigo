# The link-walk family (link-walk.R): lapply. Its closure keeps X in a
# promise, whose value holds a link, so FUN's `x[3] <<- 99` copies and
# X[[i]] reads the old vector: 1 2 3. `.Internal(lapply(x, f))` evaluates
# `x[[i]]` in the caller at each step, so it reads the binding's current
# value whether the write copies or not: 1 2 99. Before and after GNU R's
# JIT compiles the caller (its second call).
reset <- function() {
  x <<- c(1, 2, 3)
  NULL
}
reset()
print(unlist(lapply(x, function(e) { x[3] <<- 99; e })))
print(x)
reset()
print(unlist(lapply(x, function(e) { x[3] <<- 99; e })))
reset()
print(unlist(lapply(x, function(e) { x[3] <<- 99; e })))
f <- function(e) { x[3] <<- 99; e }
g <- function(...) .Internal(lapply(x, f))
reset()
print(unlist(g()))
print(x)
reset()
print(unlist(g()))
reset()
print(unlist(g()))
