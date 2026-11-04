# The value of a `{` statement is dead once the next statement starts
# (R: eval.c:do_begin keeps it in an unprotected C local), so a gc() in
# the next statement can collect an object only that value held. testthat
# evaluates each test_that() body this way: R6's finalizer tests do
# `B <- BC$new()` and then `expect_output({ rm(B); gc() }, ...)`.
# Counters, not prints, so output cannot shift between tiers.
n <- new.env(parent = globalenv())
n$k <- 0L
mk <- function() {
  e <- new.env(parent = globalenv())
  reg.finalizer(e, function(x) n$k <- n$k + 1L)
  e
}
run <- function(name, code) {
  n$k <- 0L
  r <- eval(code, new.env())
  invisible(gc())
  cat(name, ": during ", r, ", after ", n$k, "\n", sep = "")
}
run("value of the previous statement", quote({ mk(); invisible(gc()); n$k }))
run("assignment, then rm in a promise", quote({ o <- mk(); identity({ rm(o); invisible(gc()) }); n$k }))
run("assignment, then rm in a call", quote({ o <- mk(); (function(e) { rm("o", envir = e); invisible(gc()) })(environment()); n$k }))
run("nested braces", quote({ { mk() }; { invisible(gc()) }; n$k }))
# an expression vector: each element's value is dead once the next runs
n$k <- 0L
r <- eval(expression(mk(), invisible(gc()), n$k), new.env())
cat("expression vector: during ", r, "\n", sep = "")
