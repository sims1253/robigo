dyn.load("const_args.so")

# The probes: how did the constant argument arrive?
f <- function(x) .Call("capi_arg_state", environment(), quote(x))
d <- function(...) .Call("capi_dots_state", environment())

cat("top level:", f(1), d(1, "a", TRUE), "\n")
cat("eval():", eval(quote(f(1))), "\n")
cat("do.call():", do.call(f, list(1)), do.call(d, list(1, 2)), "\n")
cat("from C:", .Call("capi_c_call", f, environment()), "\n")

# A small closure at top level: interpreted at its first call, compiled
# at its second (R_CheckJIT: JIT_score below MIN_JIT_SCORE).
small <- function() f(1)
cat("small, 1st call:", small(), "\n")
cat("small, 2nd call:", small(), "\n")
cat("small, 3rd call:", small(), "\n")

# A large one is compiled at its first call.
big <- function() {
  a <- 1; b <- 2; c <- 3; dd <- 4; e <- 5; g <- 6; h <- 7; k <- 8
  a <- a + b + c + dd + e + g + h + k; b <- a * 2; c <- b - a; dd <- c * c
  e <- dd + a; g <- e - b; h <- g + c; k <- h + dd
  f(1)
}
cat("big, 1st call:", big(), "\n")

# A small closure made inside another one is never compiled.
make <- function() function() f(1)
inner <- make()
cat("inner, calls 1-3:", inner(), inner(), inner(), "\n")

# A loop at top level is compiled (R_jit_enabled > 2).
for (i in 1) cat("top-level for:", f(1), d(2), "\n")
r <- NULL; i <- 0
while (i < 1) { r <- f(1); i <- i + 1 }
cat("top-level while:", r, "\n")
r <- NULL
repeat { r <- f(1); break }
cat("top-level repeat:", r, "\n")

# Inside a compiled closure: eval() and C interpret, local() is inlined,
# promise code is compiled with its caller.
id <- function(z) z
comp <- function() {
  a <- 1; b <- 2; c <- 3; dd <- 4; e <- 5; g <- 6; h <- 7; k <- 8
  a <- a + b + c + dd + e + g + h + k; b <- a * 2; c <- b - a; dd <- c * c
  e <- dd + a; g <- e - b; h <- g + c; k <- h + dd
  c(direct = f(1), eval = eval(quote(f(1))), local = local(f(1)),
    promise = id(f(1)), c_call = .Call("capi_c_call", f, environment()),
    dots = paste(d(1, 2), collapse = "/"))
}
print(comp())
# A loop inside eval() at top level is compiled too (rho is the global env).
eval(quote(for (i in 1) cat("eval'd top-level for:", f(1), "\n")))
# A loop inside a function is part of that function's code.
loopy <- function() { for (i in 1) r <- f(1); r }
cat("small closure with a loop, 1st call:", loopy(), "\n")
