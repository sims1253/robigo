# Cost per operation of loop bodies (ns), measured inside the runtime.
# Loops inside system.time() are promises: GNU R interprets them without
# its bytecode compiler, so this compares against its AST interpreter.
x <- 1:20000000
s <- 0
t0 <- system.time(for (i in x) NULL)[[3]]
t1 <- system.time(for (i in x) { s })[[3]]
t5 <- system.time(for (i in x) { s; s; s; s; s })[[3]]
a1 <- system.time(for (i in x) { s + 1 })[[3]]
a2 <- system.time(for (i in x) { s + 1; s + 1 })[[3]]
w1 <- system.time(for (i in x) { s <- 1 })[[3]]
w2 <- system.time(for (i in x) { s <- 1; s <- 1 })[[3]]
ns <- function(t) round(t * 50, 1)
cat("empty loop/iter ns:", ns(t0), "\n")
cat("GetVar ns:", ns((t5 - t1) / 4), "\n")
cat("s+1 ns:", ns(a2 - a1), "\n")
cat("s<-1 ns:", ns(w2 - w1), "\n")
