# KNOWN DIFFERENCE (#976; not ratcheted): GNU R's cmp.R, run as R code by
# compiler::cmpfun and compiler::compile, overflows GNU R's byte-code node
# stack from 1,478 nested fold calls ("node stack overflow"). robigo-rt runs
# the same R code on a node stack of another size: it compiles at 1,478, and
# at 20,000 stops with "evaluation nested too deeply" instead. Closing it
# needs a node-stack limit of GNU R's size (R_BCNODESTACKSIZE) in the VM that
# runs cmp.R. The JIT's own compile (robigo-rt's) matches GNU R at every
# depth: tests/rt/eval/deep-nesting.R.
nest <- function(n) { d <- 1; for (i in seq_len(n)) d <- call("+", 1, d); d }
for (n in c(1477L, 1478L, 20000L)) {
  f <- function() NULL
  body(f) <- nest(n)
  g <- tryCatch(compiler::cmpfun(f), error = function(e) conditionMessage(e))
  cat(n, "cmpfun:", if (is.character(g)) g else typeof(.Internal(bodyCode(g))), "\n")
  r <- tryCatch(compiler::compile(nest(n)), error = function(e) conditionMessage(e))
  cat(n, "compile:", if (is.character(r)) r else typeof(r), "\n")
}
