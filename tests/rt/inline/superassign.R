# `<<-` and invisible() in closures GNU R's JIT never compiles (small, and
# defined inside a function, so GNU R runs their AST): robigo runs their
# calls from compiled code inline, each `<<-` buffered and written when the
# run ends. A run that gives up after a write (a warning, an error, a read
# the write could change) drops it, and the real call writes once. Each
# case runs three times from a compiled closure.
acct <- function() {
  bal <- 0
  list(dep = function(x) { bal <<- bal + x; invisible(bal) }, get = function() bal)
}
a <- acct()
for (i in 1:4000) a$dep(i %% 7)
print(a$get())
# invisible through a compiled caller at top level: nothing printed
f <- function() a$dep(1)
f(); f(); f()
g <- function() { a$dep(1); "visible" }
g(); g(); g()
print(a$get())
# the value returned is the binding's: changing it copies
h <- function() { r <- a$dep(1); r[1] <- -1; c(r, a$get()) }
for (k in 1:3) print(h())
# two writes, the second reading the first
pair <- function() {
  n <- 0L; s <- 0
  list(add = function(x) { n <<- n + 1L; s <<- s + x * n; invisible(s) }, get = function() c(n, s))
}
p <- pair()
run2 <- function() { for (i in 1:50) p$add(i); p$get() }
for (k in 1:3) print(run2())
# a write, then an integer-overflow warning: written once per call
ov <- function() {
  bal <- 0L
  list(dep = function(x) { bal <<- bal + 1L; invisible(x + 2147483647L) }, get = function() bal)
}
o <- ov()
run3 <- function() {
  for (i in 1:3) w <- tryCatch(o$dep(1L), warning = function(w) w)
  list(conditionMessage(w), conditionCall(w), o$get())
}
for (k in 1:3) print(run3())
# a write, then an error
bad <- function() {
  bal <- 0
  list(dep = function(x) { bal <<- bal + 1; invisible(x + "a") }, get = function() bal)
}
b <- bad()
run4 <- function() {
  for (i in 1:3) e <- tryCatch(b$dep(1), error = function(e) e)
  list(conditionMessage(e), conditionCall(e), b$get())
}
for (k in 1:3) print(run4())
# a global written, and one the first write defines
glob <- function() function(x) { tot <<- tot + x; invisible(tot) }
tot <- 0
gg <- glob()
run5 <- function() { for (i in 1:20) gg(i); tot }
for (k in 1:3) print(run5())
fresh <- function() function(x) { newglob <<- x; invisible(x) }
fr <- fresh()
run6 <- function() { for (i in 1:5) fr(i); newglob }
for (k in 1:3) print(run6())
# a local of the caller with the written name is not the binding written
outer <- function() { bal <- 100; d <- a$dep; for (i in 1:3) d(1); bal }
for (k in 1:3) print(outer())
print(a$get())
# the caller's argument, forced after a write of the variable it reads
cnt <- function() list(bump = function(x) { k <<- k + 1; invisible(x) })
c1 <- cnt()
run7 <- function() { k <<- 0; r <- NULL; for (i in 1:3) r <- c(r, c1$bump(k)); r }
for (j in 1:3) print(run7())
