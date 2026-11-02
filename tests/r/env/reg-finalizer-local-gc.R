# Gap: a finalizer must run during the gc(full = TRUE) that follows the
# object's scope ending. robigo still sees the object reachable (the
# dead `local()` frame's value register), so the finalizer fires at a
# later gc instead (R6's finalizer tests). Counters, not prints, so the
# output cannot shift between the interpreted, compiled and JIT tiers.
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
