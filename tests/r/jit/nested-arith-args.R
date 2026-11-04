# Arguments that are arithmetic of arithmetic (`2L * item - 1L`, `-i`) are
# evaluated eagerly by inline and native calls, where that has no effect.

# A tree built by recursion with nested arithmetic arguments.
tree <- function(item, depth) {
    if (depth == 0L)
        return(c(item, NA, NA))
    list(item, tree(2L * item - 1L, depth - 1L), tree(2L * item, depth - 1L))
}
check <- function(t)
    if (is.na(t[[2]][[1]])) t[[1]] else t[[1]] + check(t[[2]]) - check(t[[3]])
s <- 0L
for (i in 1:600) s <- s + check(tree(i, 4L)) + check(tree(-i, 4L))
s
for (i in c(1.5, -2.25)) print(check(tree(i, 3)))

# Pure scalar closures with nested and unary arguments.
f <- function(a, b) a * 10 + b
g <- function(x, y) f(-x, 2 * y - 1) + f(x %/% 2L + 1L, -(y + 1))
acc <- 0
for (i in 1:2000) acc <- acc + g(i, i / 4)
acc
r <- integer(0)
for (i in 1:1000) r[i] <- g(i, 3L)
sum(r)

# Unary minus: signed zeros, NA, logicals.
h <- function(x) 1 / x
neg0 <- function(x) h(-x)
for (i in 1:600) z <- neg0(0)
z
for (i in 1:600) z <- neg0(-0)
z
n2 <- function(x) f(-x, -(x * 1L))
for (i in 1:600) z <- n2(NA_integer_)
z
for (i in 1:600) z <- n2(TRUE)
z
for (i in 1:600) z <- n2(NA)
z

# Integer overflow inside a nested argument warns, once per call, from the
# promise the call forces.
big <- function(x) f(x * 2L + 1L, 0L)
for (i in 1:600) z <- big(i)
z
z <- big(.Machine$integer.max - 5L)
z

# An argument never forced has no effect, even one that would warn or fail.
lazy <- function(a, b) b
w <- function(x) lazy(x * 2L - 1L, 7)
for (i in 1:600) z <- w(i)
z
w(.Machine$integer.max)
w("a")
k <- function(x) lazy(-(x * 3), 8)
k(list())

# Errors from a forced nested argument.
strict <- function(a) a + 1
e <- function(x) strict(x * 2 - 1)
for (i in 1:600) z <- e(i)
z
tryCatch(e("a"), error = function(c) conditionMessage(c))
tryCatch(e(list(1)), error = function(c) conditionMessage(c))

# %% and %/% in nested arguments.
m <- function(x) f(x %% 3 + 1, -(x %/% 3))
for (i in 1:600) z <- m(i)
z
m(Inf)
m(-7.5)

# Rebinding an operator after the calls were compiled.
`-` <- function(e1, e2) if (missing(e2)) 100 else base::`-`(e1, e2) * 1000
z <- g(3, 2)
z
for (i in 1:5) print(neg0(2))
rm(`-`)
`*` <- function(e1, e2) base::`*`(e1, e2) + 0.5
z <- tree(1L, 1L)
print(unlist(z))
g(3, 2)
rm(`*`)
g(3, 2)
