# Scalar results of compiled closures called from compiled code: values,
# visibility, on.exit, return(), errors and condition handlers.
sq <- function(x) x * x
s <- 0
for (i in 1:5) s <- s + sq(i)
print(s)
inv <- function(x) invisible(x + 1L)
for (i in 1:2) { r <- inv(i); print(r) }
f <- function() { for (i in 1:2) inv(i) ; inv(10L) }
f()
print(f())
ex <- function(x) { on.exit(cat("exit", x, "\n")); x * 2 }
for (i in 1:2) print(ex(i))
ret <- function(x) { if (x > 1) return(x - 1); x + 100 }
for (i in 1:3) print(ret(i))
lg <- function(x) x > 2
for (i in 2:3) print(lg(i))
na <- function(x) if (x > 1) NA_integer_ else 5L
for (i in 1:2) print(na(i))
w <- function(x) { if (x > 1) warning("big"); x }
g <- function() { r <- 0; for (i in 1:3) r <- r + w(i); r }
print(withCallingHandlers(g(), warning = function(cond) { cat("handler:", sq(3), "\n"); invokeRestart("muffleWarning") }))
e <- function(x) if (x > 2) stop("too big") else x
h <- function() { r <- 0; for (i in 1:4) r <- r + e(i); r }
print(tryCatch(h(), error = function(cond) sq(7)))
fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
print(fib(18))
nm <- function(x) c(a = x)
for (i in 1:2) print(nm(i))
dbl <- function(x) as.numeric(x) / 3
for (i in 1:2) print(dbl(i), digits = 17)
k <- function(x) { sys.function(); x }
for (i in 1:2) print(k(i))
rc <- function(n) if (n <= 1) 1 else n * Recall(n - 1)
print(rc(10))
