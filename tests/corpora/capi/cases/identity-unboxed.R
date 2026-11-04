dyn.load("identity.so")

# GNU R's byte code keeps a reassigned scalar variable in an unboxed
# binding cell (eval.c SETVAR: NEW_BNDCELL_IVAL; the loop variable too:
# STEPFOR), and GETVAR pushes its value unboxed without boxing the cell
# (DO_GETVAR), so a builtin's argument or a compiled promise's value is a
# box of its own and C's write is not seen. Only a read through the
# binding (findVar, the interpreter) boxes the cell into the variable's
# object (envir.c BINDING_VALUE -> memory.c R_expand_binding_value).
# These functions have loops, so GNU R's JIT compiles them first.
set <- function(x, v) .Call("capi_id_set", x, v)
loopy <- function() {
  s <- 0L
  for (k in 1:300) {
    i <- k
    .Call("capi_id_set", i, 1L)
    s <- s + i
  }
  s
}
print(loopy())
forvar <- function() {
  for (k in 1:3) .Call("capi_id_set", k, 50L)
  k
}
print(forvar())
forpromise <- function() {
  for (k in 1:3) set(k, 50L)
  k
}
print(forpromise())
reassigned <- function() {
  for (r in 1) {
    i <- 0L
    i <- i + 1L
    .Call("capi_id_set", i, 5L)
  }
  i
}
print(reassigned())
# boxed by findVar first: then C's writes are seen
expanded <- function() {
  for (r in 1) {
    i <- 0L
    i <- i + 1L
    .Call("capi_id_findvar", quote(i), environment(), 42L)
    .Call("capi_id_set", i, 8L)
  }
  i
}
print(expanded())

# a logical variable: GNU R's parser gives each TRUE its own object
# (gram.c mkTrue), robigo binds the shared TRUE, which C gets as a copy
lg <- function() {
  b <- TRUE
  .Call("capi_id_set", b, FALSE)
  b
}
print(lg())
