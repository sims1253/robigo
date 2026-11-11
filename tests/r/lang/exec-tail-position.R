# Exec/Tailcall only jump from a call in tail position (eval.c:
# checkTailPosition); elsewhere they evaluate the call and carry on.
# robigo jumps from any position (gap, issue #280).
f <- function() { Exec(quote(1 + 2)); "after" }
print(f())
g <- function() { Tailcall(sum, 1, 2); "after" }
print(g())
