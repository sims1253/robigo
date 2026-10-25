# Variables cached by the bytecode VM stay observable to all other code.
# values written in a loop are visible to code run in between
s <- 0
for (i in 1:3) { s <- s + i; print(get("s")) }
s; i
f <- function() { t <- 1; for (k in 1:4) { t <- t * 2; if (k == 2) print(environment()$t) }; t }
f()
# functions assigned in a loop are used right away
for (j in 1:2) { g <- function(x) x + j; print(g(10)) }
# locked bindings still error
e <- new.env(); assign("v", 1, e); lockBinding("v", e)
r <- tryCatch(local({ for (q in 1:2) v <- q }, envir = e), error = function(c) conditionMessage(c))
r
# active bindings are called on each read and write
cnt <- 0
makeActiveBinding("ab", function(v) if (missing(v)) { cnt <<- cnt + 1; cnt } else cat("set", v, "\n"), environment())
# (reads are left out: GNU R's JIT-compiled loops call the getter twice)
for (z in 1:3) ab <- z
w <- ab; w; cnt
# promises: arguments read in loops
h <- function(a, b) { acc <- 0; for (u in 1:3) acc <- acc + a * b; acc }
h(2, { cat("forced b\n"); 5 })
# a warning handler sees the current values
x <- 0L
withCallingHandlers({ for (y in 1:3) { x <- x + 1L; if (y == 2) x <- x + .Machine$integer.max } ; x },
  warning = function(w) { cat("in handler x =", x, "\n"); invokeRestart("muffleWarning") })
# break/next through the interpreter
out <- 0
for (i in 1:10) { if (i %% 2 == 0) next; if (i > 7) break; out <- out + i }
out
# while with condition on cached variable
n <- 0; while (n < 5) n <- n + 1; n
k <- 10; repeat { k <- k - 3; if (k < 0) break }; k
