# Frames whose only extra references are their own bindings (an unforced
# default argument, a local closure, the environment bound in itself) are
# freed when the call returns; anything else that reaches the frame must
# keep it, bindings and all.
f1 <- function(x, d = c(1, 2)) x
for (i in 1:3) print(f1(i))

# The default escapes through a returned closure: it must still be forced.
f2 <- function(x, d = x * 10) function() d
g <- f2(4)
print(g())

# The frame escapes as a value.
f3 <- function(a = b + 1, b = 2) environment()
e <- f3()
print(sort(ls(e)))
print(e$a)

# A local closure returned: its frame keeps its bindings.
f4 <- function(n) { k <- n * 2; h <- function() k; h }
h <- f4(5)
print(h())

# A local closure stored elsewhere.
keep <- new.env()
f5 <- function(n, unused = stop("never")) { m <- n + 1; keep$fn <- function() m; invisible(NULL) }
f5(7)
print(keep$fn())

# A default promise passed on unforced and forced after the call returns.
f6 <- function(x = y * 3, y = 2) list(p = environment())
r <- f6()
print(get("x", envir = r$p))

# Self-bound environment.
f7 <- function() { self <- environment(); z <- 3; 1 }
print(f7())

# on.exit sees the bindings.
f8 <- function(a, d = a + 1) { h <- function() d; on.exit(print(h())); a }
print(f8(2))

# sys.function / sys.frames during the call.
f9 <- function(x, d = c("p", "q")) { fr <- sys.frame(sys.nframe()); identical(fr, environment()) }
print(f9(1))

# delayedAssign promise referring to the frame, captured outside.
holder <- new.env()
f10 <- function() { v <- 11; delayedAssign("w", v + 1, assign.env = holder); 0 }
f10()
print(holder$w)

# reg.finalizer on a frame keeps it until collected.
f11 <- function(d = 1 + 1) { reg.finalizer(environment(), function(e) cat("finalized\n")); 0 }
f11()
invisible(gc())

# Repeated calls stay correct.
s <- 0
f12 <- function(x, method = c("a", "b"), w = function() x) { method <- match.arg(method); if (method == "a") x else -x }
for (i in 1:1000) s <- s + f12(i)
print(s)
