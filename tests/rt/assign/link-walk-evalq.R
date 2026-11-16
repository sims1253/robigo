# The link-walk family (link-walk.R): evalq with a list or an environment,
# as eval (link-walk-eval.R): the evaluation frame binds the old vector.
reset <- function() {
  x <<- c(1, 2, 3)
  NULL
}
reset()
print(evalq({ x[3] <<- 99; x }, list(x = x)))
print(x)
reset()
e <- new.env()
assign("x", x, envir = e)
print(evalq({ x[3] <<- 99; x }, e))
print(x)
