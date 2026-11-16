# The link-walk family (link-walk.R): values the evaluator holds count.
# `x + f()`: x is pending while f() runs (byte code's node stack, the AST's
# evalList), so f's `x[3] <<- 99` copies and the sum uses the old x; the
# same for a builtin's earlier argument, c(x, f()), and in compiled code.
# A for loop links its sequence (do_for's INCREMENT_LINKS), so writing x in
# the body copies and the loop goes on over 1 2 3.
reset <- function() {
  x <<- c(1, 2, 3)
  NULL
}
f <- function() { x[3] <<- 99; 0 }
reset()
print(x + f())
print(x)
reset()
print(c(x, f()))
print(x)
g <- function() x + f()
reset()
print(g())
reset()
print(g())
reset()
print(g())
h <- function() {
  s <- 0
  for (e in x) { x[3] <<- 99; s <- s + e }
  s
}
reset()
print(h())
print(x)
reset()
for (e in x) { x[3] <- 99; print(e) }
print(x)
