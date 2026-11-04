# Native loops whose variables hold an integer on some paths and a double
# on others, as the binary search of the shootout's fasta_naive_2:
# `lo <- 1L; hi <- n; ... mid <- (lo + hi - 1L) %/% 2; hi <- mid`.

psum <- cumsum(c(0.27, 0.12, 0.12, 0.27, rep(0.02, 11)))
n <- length(psum)
seed <- 42L
pick <- integer(400)
types <- character(400)
for (i in 1:400) {
    seed <- (seed * 3877L + 29573L) %% 139968L
    r <- seed / 139968
    lo <- 1L
    hi <- n
    while (lo < hi) {
        mid <- (lo + hi - 1L) %/% 2
        if (psum[[mid]] >= r) hi <- mid else lo <- mid + 1L
    }
    pick[[i]] <- hi
    if (i %% 50L == 0L) types[[i]] <- typeof(hi)
}
print(table(pick))
print(c(typeof(lo), typeof(hi), typeof(mid)))

# The type after the loop is that of the last assignment.
f <- function(k) {
    x <- 0L
    for (i in 1:k) {
        if (i %% 3L == 0L) x <- x + 0.5 else x <- x + 1L
    }
    x
}
for (k in c(20L, 21L, 22L, 23L)) print(c(f(k), typeof(f(k))))
g <- function(k) {
    x <- 1.5
    for (i in 1:k) {
        if (i == k) x <- 7L else x <- x * 2
    }
    x
}
print(c(g(30), typeof(g(30))))

# Integer arithmetic while integer: overflow, %/% and %% by zero, NA.
h <- function(start, k, at) {
    x <- start
    for (i in 1:k) {
        x <- x + 1L
        if (i == at) x <- x * 1.0
    }
    x
}
print(h(.Machine$integer.max - 25L, 30L, 100L))
print(h(.Machine$integer.max - 25L, 30L, 10L))
dz <- function(den) {
    x <- 0L
    out <- double(30)
    for (i in 1:30) {
        if (i %% 2L == 0L) x <- i else x <- i + 0.25
        if (i == 25L) out[[i]] <- x %/% den else out[[i]] <- x %% 7L
    }
    c(out[24:26], x)
}
print(dz(0L))
print(dz(0))
print(dz(2L))
mz <- function() {
    y <- 3L
    for (i in 1:30) {
        if (i < 29L) y <- y + 0L else y <- y * 1.5
        z <- y %% 0L
    }
    c(y, z)
}
print(mz())
nai <- function() {
    x <- 1L
    for (i in 1:30) {
        if (i == 20L) x <- NA_integer_ else if (i %% 2L == 0L) x <- x + 1L else x <- x + 0.5
    }
    x
}
print(nai())
nad <- function() {
    x <- 1L
    for (i in 1:30) {
        if (i == 20L) x <- NA_real_ else if (i %% 2L == 0L) x <- x + 1L else x <- x * 1
    }
    x
}
print(nad())

# Used as indices, compared, stored into vectors.
v <- seq(10, 300, by = 10)
iv <- integer(30)
dv <- double(30)
for (i in 1:30) {
    if (i %% 2L == 0L) j <- i else j <- i * 1.0
    dv[[i]] <- v[[j]] + (j > 15L) + (j <= 7.5)
    iv[i] <- i
}
print(dv)
w <- integer(30)
print(tryCatch({
    for (i in 1:30) {
        if (i %% 2L == 0L) j <- i else j <- i * 1.0
        w[[i]] <- j
    }
    typeof(w)
}, error = function(e) conditionMessage(e)))

# Logicals, negation, the loop variable: not numbers.
lg <- function() {
    x <- 1L
    for (i in 1:30) if (i == 10L) x <- TRUE else x <- x + 1.5
    x
}
print(lg())
ng <- function() {
    x <- 1L
    for (i in 1:30) if (i %% 2L == 0L) x <- -x else x <- x + 0.5
    x
}
print(ng())
lv <- function() {
    s <- 0
    for (i in 1:30) {
        i <- i + 0.5
        s <- s + i
    }
    c(s, i)
}
print(lv())

# Division and powers are doubles.
dp <- function() {
    x <- 2L
    for (i in 1:30) if (i %% 2L == 0L) x <- x / 2L else x <- (x %/% 1)^1L
    c(x, typeof(x))
}
print(dp())

# No negative zero while integer: 0L * -1L is 0L.
nz <- function() {
    x <- 0L
    y <- double(30)
    for (i in 1:30) {
        if (i %% 2L == 0L) x <- 0L else x <- 0
        x <- x * -1L
        y[[i]] <- 1 / x
    }
    c(y[[29]], y[[30]])
}
print(nz())
