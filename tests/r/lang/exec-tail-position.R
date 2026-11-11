# Exec/Tailcall only jump from a call in tail position (eval.c:
# checkTailPosition); elsewhere they evaluate the call and carry on.
f <- function() { Exec(quote(1 + 2)); "after" }
print(f())
g <- function() { Tailcall(sum, 1, 2); "after" }
print(g())
h <- function(n) if (n > 0) Tailcall(h, n - 1) else "bottom"
print(h(3))
k <- function(n) { x <- 1; if (n > 0) { Tailcall(k, n - 1) } else n }
print(k(4))
m <- function() { y <- Tailcall(sum, 2); y + 10 }
print(m())
w <- function() { Exec(quote(sys.function())); 7 }
print(is.function(w()))
