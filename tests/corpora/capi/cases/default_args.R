dyn.load("default_args.so")
st <- function(env, ...) .Call("capi_binding_states", env, c(...))

# Constant defaults of each kind, and a call, in an interpreted closure.
f <- function(a = 1, b = 2L, c = "x", d = TRUE, e = NULL, g = 1i, h = c(1, 2), k)
  st(environment(), "a", "b", "c", "d", "e", "g", "h", "k")
print(f())
# A supplied argument is the call's: a promise of `k`'s expression.
print(f(k = sqrt(4)))

# The same in a closure the JIT compiles at its first call.
big <- function(a = 1, b = "x", h = c(1, 2)) {
  p <- 1; q <- 2; r <- 3; s <- 4; t <- 5; u <- 6; v <- 7; w <- 8
  p <- p + q + r + s + t + u + v + w; q <- p * 2; r <- q - p; s <- r * r
  t <- s + p; u <- t - q; v <- u + r; w <- v + s
  st(environment(), "a", "b", "h")
}
print(big())

# Forcing a default leaves a forced promise.
forced <- function(a = 1) { a; st(environment(), "a") }
print(forced())

# missing() and substitute() see the default as before.
m <- function(a = 1) c(missing(a), deparse(substitute(a)), a)
print(m())
print(m(5))

