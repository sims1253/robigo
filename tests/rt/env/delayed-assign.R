# assign() and delayedAssign() into a closure frame, as the D2S decision
# rows use them (benchmarks/rt/calls-rows.toml, group d2s)
f <- function() { assign("v", 1); v }
cat(f(), "\n")
setv <- function() assign("v", 2, envir = parent.frame())
g <- function() { setv(); v }
cat(g(), "\n")
h <- function(e) delayedAssign("v", { cat("forced\n"); 3 }, assign.env = e)
k <- function() { h(environment()); cat("before\n"); c(v, v) }
cat(k(), "\n")
m <- function() { eval(quote(v <- 4)); v }
cat(m(), "\n")
# the promise evaluates in eval.env, defaulting to the caller
x <- "global"
p <- function() { x <- "local"; delayedAssign("y", x); y }
cat(p(), "\n")
q <- function() { x <- "local"; delayedAssign("y", x, eval.env = globalenv()); y }
cat(q(), "\n")
print(delayedAssign("z", 1))
r1 <- tryCatch(delayedAssign(character(), 1), error = conditionMessage)
cat(r1, "\n")
r2 <- tryCatch(delayedAssign("z", 1, eval.env = 1), error = conditionMessage)
cat(r2, "\n")
r3 <- tryCatch(delayedAssign("z", 1, assign.env = list()), error = conditionMessage)
cat(r3, "\n")
r4 <- tryCatch(delayedAssign("z", 1, assign.env = NULL), error = conditionMessage)
cat(r4, "\n")
