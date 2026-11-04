# A closure that calls one of base's simple-.Internal wrappers
# (identical, inherits, typeof, ...: compiler/R/cmp.R:safeBaseInternals)
# must not stay alive through robigo's inlining memo: GNU R inlines at
# compile time and keeps no reference to the caller. R6's clone() calls
# identical() in a loop inside a closure, and the objects it cloned were
# never finalized. Counters, not prints, so output cannot shift between
# the interpreted, compiled and JIT tiers.
n <- new.env(parent = globalenv())
n$k <- 0L
mk <- function() {
  e <- new.env(parent = globalenv())
  reg.finalizer(e, function(x) n$k <- n$k + 1L)
  e
}
run <- function(name, op) {
  n$k <- 0L
  h <- function() {
    o <- mk()
    op(o)
    rm(o)
    invisible(gc())
    n$k
  }
  r <- h()
  invisible(gc())
  cat(name, ": during ", r, ", after ", n$k, "\n", sep = "")
}
# the caller is a closure made in the frame that holds the object
run("identical in a loop", function(o) {
  f <- function(x) { for (i in 1) identical(x, o); 1 }
  f(o)
})
run("identical, no loop", function(o) {
  f <- function(x) { identical(x, o); 1 }
  f(o)
})
run("inherits in a loop", function(o) {
  f <- function(x) { for (i in 1:3) inherits(x, "foo"); 1 }
  f(o)
})
run("typeof in lapply", function(o) {
  lapply(list(o), function(x) { for (i in 1) typeof(x); x })
})
# a caller hot enough to be compiled
run("identical, hot caller", function(o) {
  f <- function(x) { s <- 0; for (i in 1:200) if (identical(x, o)) s <- s + 1; s }
  for (j in 1:20) f(o)
})
