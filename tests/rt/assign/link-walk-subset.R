# The link-walk family (link-walk.R): `[[`'s default runs R code when
# exact = NA warns of a partial match, before it reads the element, and a
# calling handler can write the list it is reading. The AST's
# R_DispatchOrEvalSP drops the object's link before the default runs, so
# the write is made in place and `[[` reads it: 99 2. Byte code keeps the
# object on the node stack below the call frame (DO_STARTDISPATCH;
# BUILTIN_CALL_FRAME_ARGS drops only the argument list's links), so once
# GNU R's JIT has compiled h the write copies and `[[` reads the old
# element: 1 2. This recording holds in production only (the .requires:
# inplace-identity).
x <- list(foobar = c(1, 2), b = 3)
r <- withCallingHandlers(x[["foo", exact = NA]], warning = function(w) {
  x[["foobar"]][1] <<- 99
  invokeRestart("muffleWarning")
})
print(r)
print(x$foobar)
h <- function() {
  y <- list(foobar = c(1, 2))
  r <- withCallingHandlers(y[["foo", exact = NA]], warning = function(w) {
    y[["foobar"]][1] <<- 99
    invokeRestart("muffleWarning")
  })
  print(y$foobar)
  r
}
print(h())
print(h())
print(h())
