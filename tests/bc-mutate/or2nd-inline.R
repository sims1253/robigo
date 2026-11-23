# OR2ND and AND2ND keep the first operand's value where the second does not
# decide, unread (bcEval, eval.c's AND2ND/OR2ND), whatever byte code put
# there: here an integer, 2L + 3L, with no OR1ST or AND1ST before (#976,
# #1003: seed 7's case, and X's find that robigo-rt's inline runs wrote the
# second operand over it). Called directly (the VM) and from a compiled
# caller's loop (where robigo-rt runs them inline: ADD and LT/GT leave
# unboxed operands there), the values are GNU R 4.6.2's: 5L, 5L, TRUE,
# FALSE, and 44L for each h(4). scripts/rt-bc-mutate.py runs it once; it
# prints "or2nd ok". (Not a conformance case: the crosscheck compares a
# translation with its expression's compile, and this byte code is no
# expression's compile.)
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
stopifnot(identical(list(or2(), and2(), or2t(), and2f()), list(5L, 5L, TRUE, FALSE)))
h <- function(n) { s <- 0L; for (i in 1:n) s <- s + or2() + and2() + or2t() + and2f(); s }
for (k in 1:4) stopifnot(identical(h(4), 44L))
cat("or2nd ok\n")
