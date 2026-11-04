# A finalizer must run during the gc(full = TRUE) that follows the
# object's scope ending: the `local()` call's value is dead once the
# statement is done (R: bcEval POP; the VM clears its register, #357), so
# it does not keep the object reachable (R6's finalizer tests). Counters,
# not prints, so the output cannot shift between the interpreted,
# compiled and JIT tiers.
n <- new.env(parent = globalenv())
n$k <- 0L
mk <- function() {
  e <- new.env(parent = parent.frame())
  e$self <- function() e
  reg.finalizer(e, function(x) n$k <<- n$k + 1L)
  e
}
f <- function() {
  local(obj <- mk())
  invisible(gc(full = TRUE))
  n$k
}
cat("during:", f(), "\n")
invisible(gc())
invisible(gc())
cat("after:", n$k, "\n")
