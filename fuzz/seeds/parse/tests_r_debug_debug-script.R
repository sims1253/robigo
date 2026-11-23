# debug(), debugonce() and undebug() read the browser's commands from the
# script.
g <- function(fun) fun(1)
debug(g)
isdebugged(g)
g(function(x) x + 1)
c
g(function(x) x * 10)
n
undebug(g)
isdebugged(g)
g(function(x) x - 1)
# A braced body is stepped statement by statement.
f <- function(x) {
  y <- x + 1
  z <- y * 2
  z
}
debugonce(f)
f(1)
n
y
n
n
n
f(2)
# s steps into the functions it calls.
inner <- function(u) {
  u + 100
}
outer <- function(u) {
  w <- inner(u)
  w
}
debugonce(outer)
outer(1)
n
s
n
n
n
# c inside a loop of a debugged function
lf <- function(n) {
  tot <- 0
  for (i in seq_len(n)) tot <- tot + i
  while (tot > 4) tot <- tot - 2
  tot
}
debugonce(lf)
lf(3)
n
n
n
n
c
c
cat("loops done\n")
# Q quits the session.
debugonce(f)
f(3)
Q
cat("not reached\n")
