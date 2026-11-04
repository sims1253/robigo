# Native loops: integer indices (NA, zero, negative, past the end leave
# native code) and integer %% (negative operands, zero, NA).
x <- as.double(1:50)
rd <- function(idx) {
    s <- 0
    for (k in 1:length(idx)) {
        j <- idx[[k]]
        s <- s + x[[j]]
    }
    s
}
print(rd(rep(c(1L, 50L, 25L), 20)))
for (bad in list(c(rep(1L, 30), NA), c(rep(2L, 30), 0L), c(rep(3L, 30), -1L), c(rep(4L, 30), 51L)))
    print(tryCatch(rd(bad), error = function(e) conditionMessage(e)))
wr <- function(idx) {
    v <- double(10)
    for (k in 1:length(idx)) {
        j <- idx[[k]]
        v[j] <- k
    }
    v
}
print(wr(rep(1:10, 3)))
print(wr(c(rep(1:10, 3), 12L)))
print(tryCatch(wr(c(rep(1:10, 3), NA)), error = function(e) conditionMessage(e)))
print(wr(c(rep(1:10, 3), 0L)))
m <- matrix(as.double(1:12), 3)
mr <- function(i, j) {
    s <- 0
    for (k in 1:length(i)) {
        r <- i[[k]]
        c <- j[[k]]
        s <- s + m[[r, c]]
    }
    s
}
print(mr(rep(1:3, 10), rep(1:4, length.out = 30)))
print(tryCatch(mr(c(rep(1:3, 10), 4L), c(rep(1L, 30), 1L)), error = function(e) conditionMessage(e)))
md <- function(a, b) {
    out <- integer(length(a))
    for (k in 1:length(a)) {
        p <- a[[k]]
        q <- b[[k]]
        out[[k]] <- p %% q
    }
    out
}
a <- c(7L, -7L, 7L, -7L, 0L, 2147483647L, NA, 5L, -2147483647L)
b <- c(3L, 3L, -3L, -3L, 5L, 2L, 3L, NA, 7L)
print(md(rep(a, 3), rep(b, 3)))
print(md(c(rep(10L, 30), 4L), c(rep(3L, 30), 0L)))
gen <- function(n) {
    r <- 42L
    s <- 0L
    for (k in 1:n) {
        r <- (r * 3877L + 29573L) %% 139968L
        s <- (s + r) %% 1000L
    }
    c(r, s)
}
print(gen(5000))
