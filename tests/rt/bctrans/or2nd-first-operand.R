# OR2ND and AND2ND keep the first operand's value where the second does not
# decide, unread (bcEval, eval.c's AND2ND/OR2ND), whatever byte code put
# there: here an integer, 2L + 3L, with no OR1ST or AND1ST before (#976,
# #1003's seed 7 case). The value is the same when the closure is called
# directly and from a compiled caller's loop (where robigo-rt runs it
# inline: ADD and LT/GT leave unboxed operands there).
# LDCONST 0, LDCONST 1, ADD 2, LDCONST 1, LDCONST 3, LT/GT 4,
# OR2ND/AND2ND 5, RETURN
mk <- function(op, rel) .Internal(bcClose(NULL, .Internal(mkCode(
  c(12L, 16L, 0L, 16L, 1L, 44L, 2L, 16L, 1L, 16L, 3L, rel, 4L, op, 5L, 1L),
  list(2L, 3L, quote(2L + 3L), 0L, quote(3L > 0L), quote(a || b)))), globalenv()))
or2 <- mk(91L, 53L)
and2 <- mk(89L, 56L)
# the second operand decides: its value
or2t <- mk(91L, 56L)
and2f <- mk(89L, 53L)
print(list(or2(), and2(), or2t(), and2f()))
h <- function(n) { s <- 0L; for (i in 1:n) s <- s + or2() + and2() + or2t() + and2f(); s }
for (k in 1:4) print(h(4))
