# When GNU R's JIT compiles a closure, base functions it found are
# inlined: later rebinding them is not seen by that compiled code. Each
# case uses its own function body (the JIT cache is keyed by body).
lt <- function(a, b) { cat("user <\n"); base::`<`(a, b) }

# 1. A small top-level closure, `<` rebound before its first call.
f1 <- function(n) if (n < 2) "small" else "big"
`<` <- lt
for (k in 1:3) print(f1(5))
rm(`<`)

# 2. Rebound after the first call.
f2 <- function(n) if (n < 3) "small" else "big"
print(f2(5))
`<` <- lt
for (k in 1:3) print(f2(5))
rm(`<`)

# 3. Rebound after the second call.
f3 <- function(n) if (n < 4) "small" else "big"
print(f3(5)); print(f3(5))
`<` <- lt
for (k in 1:2) print(f3(5))
rm(`<`)

# 4. A closure with a loop (compiled at its first call).
f4 <- function(n) { s <- 0; for (i in 1:n) if (i < 3) s <- s + 1; s }
print(f4(4))
`<` <- lt
print(f4(4))
rm(`<`)

# 5. A closure defined inside a function call.
mk <- function() function(n) if (n < 5) "small" else "big"
f5 <- mk()
print(f5(9))
`<` <- lt
for (k in 1:2) print(f5(9))
rm(`<`)

# 6. Two closures with the same body: the second reuses the first's code.
f6a <- function(n) if (n < 6) "small" else "big"
for (k in 1:2) print(f6a(9))
`<` <- lt
f6b <- function(n) if (n < 6) "small" else "big"
for (k in 1:2) print(f6b(9))
rm(`<`)

# 7. `+` rebound in the closure's own environment before compilation.
mk7 <- function() { `+` <- function(a, b) { cat("local +\n"); base::`+`(a, b) }; function(x) x + 1 }
f7 <- mk7()
for (k in 1:3) print(f7(1))

# 8. A non-operator base function (guarded inlining in GNU R).
f8 <- function(x) length(x)
for (k in 1:2) print(f8(1:3))
length <- function(x) { cat("user length\n"); 99L }
for (k in 1:2) print(f8(1:3))
rm(length)
