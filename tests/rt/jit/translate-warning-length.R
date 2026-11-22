# A character assignment target of length other than 1 whose first name
# does not translate (LC_ALL=C, the .locale sidecar). GNU R's checkAssign
# leaves it to the special (cmp.R's cmpSpecial), so do_set warns at each
# evaluation, compiled or not. Under byte code the warning's call is
# R_getBCInterpreterExpression's: an assignment call of a name of more than
# two bytes ending in `<-` (`<<-`) is shown as itself, `<-` and `=` give
# the function's call. findLocals' addVar of the whole vector warns too
# (do_assign: only the first element is used).
options(warn = 1)
nm <- c("\u4e2d", "x")
mk <- function(op, loop) {
  body <- if (loop) substitute({ OP(N, 1); for (i in 1) NULL; 0 }, list(OP = as.name(op), N = nm))
          else substitute({ OP(N, 1); 0 }, list(OP = as.name(op), N = nm))
  f <- function() NULL
  body(f) <- body
  f
}
for (op in c("<<-", "<-", "=")) {
  message("interpreted ", op)
  f <- mk(op, FALSE); print(f())
  message("compiled ", op)
  f <- mk(op, TRUE); print(f()); print(f())
}
