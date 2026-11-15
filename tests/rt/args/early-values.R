# Arguments a compiled caller may compute before the callee reads them
# (robigo-rt's early values): whatever happens between the call and the
# read, the value is the one GNU R's evaluation gives at the read. Each
# case runs in a loop so that its argument's code is compiled by the
# first call and computed early by the later ones.
rep3 <- function(f) for (k in 1:3) print(f())

# the callee forces another argument first, which assigns in the caller
f1 <- function(x, y) { y; x }
g1 <- function() { i <- 1; f1(i * 2, i <- 5) }
rep3(g1)

# the callee assigns the caller's variable with <<-
i2 <- 1
f2 <- function(x) { i2 <<- i2 + 10; x }
g2 <- function() f2(i2 * 2)
rep3(g2)

# a builtin assigns it
f3 <- function(x, e) { assign("i3", 7, envir = e); x }
g3 <- function() { i3 <- 1; f3(i3 + 1, environment()) }
rep3(g3)

# a closure the callee calls assigns it
set4 <- function() i4 <<- i4 * 3
i4 <- 2
f4 <- function(x) { set4(); x }
g4 <- function() f4(i4 - 1)
rep3(g4)

# the callee captures its frame and returns before the read
f5 <- function(x) function() x
g5 <- function() { i5 <- 1; h <- f5(i5 * 4); i5 <- 100; h() }
rep3(g5)

# a symbol argument, its variable reassigned through an environment
f6 <- function(x, e) { e$v6 <- "changed"; x }
g6 <- function() { v6 <- "original"; f6(v6, environment()) }
rep3(g6)

# the argument is never read; then it is read through substitute
f7 <- function(x) 1
g7 <- function() { i7 <- 1; f7(i7 / 0) }
rep3(g7)
f8 <- function(x) { s <- substitute(x); list(s, x) }
g8 <- function() { i8 <- 3; f8(i8 * 3) }
rep3(g8)

# missing arguments and integer overflow are not computed early
f9 <- function(x) if (missing(x)) "missing" else x
g9 <- function(a) f9(a)
for (k in 1:3) print(g9())
f10 <- function(x) x
g10 <- function() { m <- 2147483647L; f10(m + 1L) }
for (k in 1:3) print(g10())

# visibility of the value read
f11 <- function(x) x
g11 <- function() { i11 <- 2; f11(i11 * 2) }
for (k in 1:3) g11()
f12 <- function(x) invisible(x)
g12 <- function() { i12 <- 2; f12(i12 * 2) }
for (k in 1:3) g12()
g11()
g12()
g11()
g12()

# an argument read by a default and by a promise chain
f13 <- function(x, y = x * 2) y + x
g13 <- function() { i13 <- 5; f13(i13 + 1) }
rep3(g13)
f14 <- function(a) f13(a)
g14 <- function() { i14 <- 1; f14(i14 + 1) }
rep3(g14)

# recursion: each level's argument computed by its caller
fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
print(fib(15))
