# A Tailcall() or Exec() out of a debugged frame: R_execClosure reports
# "exiting from" for the frame, then applyClosure runs the continuation.
g <- function(n) n * 10
f <- function(n) {
  if (n > 0) Tailcall(f, n - 1) else Tailcall(g, 7)
}
debugonce(f)
f(2)
n
n
n
# Every frame of a debug()ged function is debugged, the replacements too.
h <- function(n) if (n > 0) Exec(quote(h(n - 1))) else "done"
debug(h)
h(2)
n
n
n
n
n
undebug(h)
h(3)
# A continuation that calls a builtin.
k <- function() {
  x <- 1
  Tailcall(sum, x, 2)
}
debugonce(k)
k()
c
# Stepping into a function that tail-calls.
inner <- function(u) Tailcall(g, u + 1)
outer <- function(u) {
  v <- inner(u)
  v + 1
}
debugonce(outer)
outer(1)
n
s
n
n
n
