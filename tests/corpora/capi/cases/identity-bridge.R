dyn.load("identity_vm.so")

# A scalar counter in a function's frame, written by C before each
# callback; the callbacks and the code after read the writes. (This needs
# the bridge to hand C the variable's own vector, not a copy written back
# at return: #364.)
count <- function(start) {
  i <- 0L
  r <- .Call("capi_id_loop", environment(), quote(i * 10L), i, start, 3L)
  print(unlist(r))
  i
}
print(count(1L))
print(count(200L))
countd <- function(start) {
  x <- 0
  r <- .Call("capi_id_loop", environment(), quote(x + 0.5), x, start, 3L)
  print(unlist(r))
  x
}
print(countd(1))
print(countd(1000.25))

# the same at top level
i <- 0L
print(unlist(.Call("capi_id_loop", environment(), quote(i + 1L), i, 130L, 2L)))
print(i)

# purrr's map: `i` reaches C as a promise through `...` and pairlist(),
# and the callbacks evaluate `.f(.x[[i]], ...)` in map's frame
call_with_cleanup <- function(...) .Call("capi_id_dots", pairlist(...))
with_indexed_errors <- function(expr, i) {
  withCallingHandlers(expr, error = function(cnd) cat("in index", i, "\n"))
}
map <- function(.x, .f, ...) {
  n <- length(.x)
  i <- 0L
  with_indexed_errors(i = i, call_with_cleanup(environment(), quote(.f(.x[[i]], ...)), i, n))
}
print(map(1:3, function(x) x * 10))
print(map(c(a = 2.5, b = 3.5), function(x, y) x + y, y = 1))
print(unlist(map(1:200, function(x) x)[c(1, 128, 200)]))
