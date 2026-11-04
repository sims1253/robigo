dyn.load("identity.so")

# C writes into a variable's value; R reads the write through the
# variable after the call. (Function bodies run on robigo's VM, top-level
# code in its interpreter. The first assignment of a variable defines a
# boxed binding in GNU R, byte code too: eval.c SETVAR -> defineVar. For
# reassigned ones see identity-unboxed.R; for writes read by callbacks
# during the call, identity-bridge.R.)

# no callback: the write is read after the call
set1 <- function() {
  i <- 0L
  .Call("capi_id_set", i, 5L)
  i
}
print(set1())
set2 <- function() {
  x <- 2.5
  .Call("capi_id_set", x, 7.5)
  x
}
print(set2())

# found by name, not passed
fv <- function() {
  i <- 5L
  .Call("capi_id_findvar", quote(i), environment(), 42L)
  i
}
print(fv())

# through an argument's promise: forced by the callee first, or lazily
setter <- function(x) {
  .Call("capi_id_set", x, 77L)
  x
}
eager <- function() {
  i <- 300L
  y <- setter(i)
  c(i, y)
}
print(eager())
lazy <- function(x, ...) {
  force(x)
  .Call("capi_id_set", x, 66L)
  x
}
lz <- function() {
  i <- 3L
  y <- lazy(i)
  c(i, y)
}
print(lz())

# copy-on-modify after C wrote
cow <- function() {
  i <- 0L
  .Call("capi_id_set", i, 7L)
  j <- i
  i <- i + 1L
  c(i, j)
}
print(cow())

# a value from a builtin: its own object
len <- function(x) {
  n <- length(x)
  .Call("capi_id_set", n, 99L)
  c(n, length(x))
}
print(len(1:3))
n <- length(1:4)
.Call("capi_id_set", n, 98L)
print(c(n, length(1:4)))

# a literal passed straight to C: the shared small integers go as copies
# whose writes are not carried back (#345), so other 0L/1L/3L are intact
.Call("capi_id_set", 0L, 9L)
print(0L)
k <- function() {
  .Call("capi_id_set", 1L, 9L)
  1L
}
print(k())
print(k())
z <- 3L
w <- function() 3L
.Call("capi_id_set", z, 4L)
print(c(z, w(), 3L))
