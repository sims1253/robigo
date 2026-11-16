# The link-walk family (link-walk.R): eval with a list or an environment.
# The evaluation frame binds the old vector (the list's element, or the
# value assign() bound), so `x[3] <<- 99` on the global x copies and `x`
# in that frame stays 1 2 3.
reset <- function() {
  x <<- c(1, 2, 3)
  NULL
}
reset()
print(eval(quote({ x[3] <<- 99; x }), list(x = x)))
print(x)
reset()
e <- new.env()
assign("x", x, envir = e)
print(eval(quote({ x[3] <<- 99; x }), e))
print(x)
