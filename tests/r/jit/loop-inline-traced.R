# A native loop compiled with a closure inline makes the calls in the VM
# once the closure is traced in place (.primTrace): each prints its trace.
f <- function(x) x + 1
g <- function(n) { s <- 0; for (i in 1:n) s <- s + f(i); s }
print(g(3000))
print(g(3000))
.primTrace(f)
print(g(3))
.primUntrace(f)
print(g(5))
