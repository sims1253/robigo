# The deepest nest of fold calls GNU R's cmp.R compiles (#976, compile.rs's
# CMP_DEPTH): `1 + (1 + ... 1)` n deep, compiled with compiler::compile;
# from one deeper cmp.R stops with "node stack overflow" (its recursion on
# the byte-code interpreter's node stack, R_BCNODESTACKSIZE). Run on the
# oracle from a checkout:
#   scripts/oracle-run.sh 4.6.2 Rscript --vanilla scripts/oracle-cmp-depth.R
# GNU R 4.6.2 (2026-11-23): compiles up to 1477, fails from 1478, and at
# 1477 the code is 4 words (LDCONST 1478; RETURN).
lo <- 1L; hi <- 20000L
ok <- function(n) {
  d <- 1
  for (i in seq_len(n)) d <- call("+", 1, d)
  !inherits(tryCatch(compiler::compile(d), error = function(e) e), "error")
}
stopifnot(ok(lo), !ok(hi))
while (hi - lo > 1) { m <- (lo + hi) %/% 2L; if (ok(m)) lo <- m else hi <- m }
d <- 1
for (i in seq_len(lo)) d <- call("+", 1, d)
message("cmp.R compiles up to ", lo, " (", length(.Internal(disassemble(compiler::compile(d)))[[2]]),
        " code words), fails from ", hi, " (", conditionMessage(tryCatch({
          for (i in 1L) d <- call("+", 1, d); compiler::compile(d) }, error = function(e) e)), ")")
