# A native loop compiled with a closure inline makes the calls in the VM
# once the closure is traced in place (.primTrace): each prints its trace.
# (The interpreter prints the call, bytecode the function's name, as in
# GNU R: only the trace lines are counted.)
lines <- function(expr) {
    tc <- textConnection("out", "w", local = TRUE)
    sink(tc)
    on.exit({ sink(); close(tc) })
    r <- expr
    list(out = out, r = r)
}
f <- function(x) x + 1
g <- function(n) { s <- 0; for (i in 1:n) s <- s + f(i); s }
print(g(3000))
print(g(3000))
.primTrace(f)
x <- lines(g(3))
print(length(grep("^trace: f", x$out)))
print(x$r)
.primUntrace(f)
x <- lines(g(5))
print(length(x$out))
print(x$r)
