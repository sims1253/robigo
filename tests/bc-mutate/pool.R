# A compile that stops midway gives its buffers back to the isolate's pool
# (#988's scratch) clean, so the next compile is correct (#976, #1003).
# scripts/rt-bc-mutate.py runs it once under gctorture, which collects at
# every allocation: a stale object left in a pooled buffer is then a wrong
# value or a crash. Each round aborts compiles three ways, then compiles
# and runs closures that must give their values.
bad <- function(code, consts) {
  g <- .Internal(bcClose(NULL, .Internal(mkCode(code, consts)), globalenv()))
  tryCatch({ g(); "ran" }, error = function(e) conditionMessage(e))
}
# a closure body past cmp.R's depth (CMP_DEPTH): its compile stops there
deep <- 1
for (i in 1:1500) deep <- call("+", 1, deep)
h <- function() NULL
body(h) <- deep
good <- function(x) { s <- 0; for (i in seq_len(x)) s <- s + i * 2; v <- c(a = s, b = x); v[["a"]] + v[["b"]] }
for (round in 1:3) {
  # GOTO (2) to a label past the code; LDCONST (16) of a constant past the pool
  m1 <- bad(c(12L, 2L, 1000L), NULL)
  m2 <- bad(c(12L, 16L, 5L, 1L), list(1))
  stopifnot(startsWith(m1, "robigo: malformed byte code"), startsWith(m2, "robigo: malformed byte code"))
  stopifnot(h() == 1501)
  stopifnot(good(10) == 120)
  # a closure first compiled after the aborts
  k <- eval(call("function", as.pairlist(alist(y = )), quote({ z <- y; for (j in 1:3) z <- z + j; z })))
  stopifnot(k(round) == round + 6)
}
cat("pool ok\n")
