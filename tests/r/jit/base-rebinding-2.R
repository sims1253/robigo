# More of GNU R's JIT binding of base functions (see base-rebinding.R):
# when a closure counts as compiled, and which bindings compilation fixes.
lt <- function(a, b) { cat("user <\n"); base::`<`(a, b) }
pl <- function(a, b) { cat("user +\n"); base::`+`(a, b) }

# 1. `if`'s JIT score is that of its condition and consequent only: a loop
#    in the alternative does not make the closure compiled at its 1st call.
f1 <- function(n) if (n < 1) 0 else { s <- 0; for (i in 1:2) s <- s + 1; s }
print(f1(3))
`<` <- lt
print(f1(3))
print(f1(3))
rm(`<`)

# 2. A closure defined in a call with a loop: compiled at its 2nd call.
mk2 <- function() function(n) { s <- 0; for (i in seq_len(n)) if (i < 2) s <- s + 1; s }
f2 <- mk2()
print(f2(3))
`<` <- lt
print(f2(3))
print(f2(3))
rm(`<`)
f2b <- mk2()
print(f2b(3))
print(f2b(3))
`<` <- lt
print(f2b(3))
rm(`<`)

# 3. Closures created by compiled code are compiled with it.
mk3 <- function() function(n) if (n < 7) "small" else "big"
g3 <- mk3()
g3 <- mk3()
`<` <- lt
print(g3(9))
h3 <- mk3()
print(h3(9))
rm(`<`)

# 4. A body that assigns `<` itself: never bound to base.
f4 <- function(n, redefine) {
  if (redefine) `<` <- lt
  n < 3
}
print(f4(1, FALSE)); print(f4(1, FALSE))
print(f4(1, TRUE))

# 5. Same body, different formals: the cached code does not fit the
#    second closure, which is then never compiled.
f5a <- function(n) if (n < 8) "small" else "big"
print(f5a(9)); print(f5a(9))
f5b <- function(n, m) if (n < 8) "small" else "big"
`<` <- lt
print(f5b(9)); print(f5b(9)); print(f5b(9))
rm(`<`)

# 6. Rebinding in the frame of a compiled closure (assign()).
f6 <- function(n) { assign("+", pl, envir = environment()); n + 1 }
print(f6(1)); print(f6(1))
print(f6(1))

# 7. Operators in arguments of calls, and in a closure created inside
#    lapply's argument.
f7 <- function(x) { print(x < 2); unlist(lapply(1:2, function(i) i + x)) }
print(f7(1)); print(f7(1))
`<` <- lt
`+` <- pl
print(f7(1))
rm(`<`, `+`)

# 8. A large body (score >= 50) without a loop: compiled at the 1st call.
f8 <- function(n) if (n < 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1) "small" else "big"
print(f8(30))
`<` <- lt
print(f8(30))
rm(`<`)
