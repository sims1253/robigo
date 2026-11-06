dyn.load("finalizer_callback.so")

# rm() and gc() in R code that C evaluates (rlang's eval_bare under
# testthat's expect_output): the object is unreachable at that gc(), so
# its finalizer runs there. It first passes through C at several stack
# depths, leaving its address in stack memory the evaluator reuses.
n <- new.env()
n$k <- 0L
fin <- function(e) n$k <- n$k + 1L
deep <- function(d, x) if (d == 0) .Call("capi_fc_touch", x) else { y <- deep(d - 1, x); y }
on_time <- 0L
for (i in 1:20) {
  n$k <- 0L
  b <- new.env()
  reg.finalizer(b, fin)
  for (d in c(0, 1, 3, 7, 15, 31)) invisible(deep(d, b))
  .Call("capi_fc_eval", quote({ rm(b); invisible(gc()) }), environment())
  on_time <- on_time + n$k
}
invisible(gc())
cat("finalized in their gc():", on_time, "of 20\n")

# the same in a function's frame
f <- function() {
  n$k <- 0L
  b <- new.env()
  reg.finalizer(b, fin)
  for (d in c(0, 2, 8, 32)) invisible(deep(d, b))
  .Call("capi_fc_eval", quote({ rm(b); invisible(gc()) }), environment())
  n$k
}
cat("in a function:", sum(vapply(1:10, function(i) f(), 0L)), "of 10\n")
