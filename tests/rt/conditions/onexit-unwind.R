# E8 (docs/g3.md §15.3), recorded on GNU R 4.6.2: on.exit code that catches
# its own error during an unwind leaves the outer condition. The Rust tests
# of crates/robigo-rtcore/tests/contexts.rs (E8) assert the same; the case
# across a C frame is tests/rt/capi/e8-unwind.R.
f <- function() {
  on.exit(tryCatch(stop("inner"), error = function(e) cat("on.exit caught:", conditionMessage(e), "\n")))
  stop("outer")
}
r <- tryCatch(f(), error = function(e) e)
print(conditionMessage(r))
print(conditionCall(r))

# a calling handler outside sees the outer error; the on.exit code's own
# error goes to the handler it set up
withCallingHandlers(
  tryCatch(f(), error = function(e) cat("exiting handler:", conditionMessage(e), "\n")),
  error = function(e) cat("calling handler:", conditionMessage(e), "\n"))

# two levels: each on.exit catches its own, and the outer error leaves
g <- function() {
  on.exit(tryCatch(stop("g's inner"), error = function(e) cat("g's on.exit caught\n")))
  f()
}
print(tryCatch(g(), error = function(e) conditionMessage(e)))

# a warning raised and muffled in on.exit code during the unwind
h <- function() {
  on.exit(withCallingHandlers(warning("in on.exit"), warning = function(w) {
    cat("muffled:", conditionMessage(w), "\n")
    invokeRestart("muffleWarning")
  }))
  stop("h's error")
}
print(tryCatch(h(), error = function(e) conditionMessage(e)))

# uncaught: the top level reports the outer error
f()
