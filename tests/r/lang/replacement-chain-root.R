# In a nested replacement f(a$b, <arg>) <- v, R code in the setter's and
# getters' arguments sees the target bound (eval.c:applydefine): a getter's
# arguments are evaluated again for its setter, after the replacement.
f <- function() {
    a <- list(b = 1)
    attr(a$b, { print(a); "hello" }) <- 42
    a
}
print(f())
g <- function() {
    a <- list(b = 1:3)
    a$b[{ print(a); 2 }] <- 42L
    a
}
print(g())
h <- function() {
    a <- list(b = list(c = 1))
    a[[{ print(a$b$c); "b" }]]$c <- 2
    a
}
print(h())
k <- function() {
    a <- list(b = 1:3)
    withCallingHandlers(a$b[as.integer("x")] <- 1L,
        warning = function(w) { print(a); invokeRestart("muffleWarning") })
    a
}
print(k())
# an index that rebinds the target: the assignment still wins
r1 <- function() {
    a <- list(b = 1:3)
    a$b[{ a <- "gone"; 2 }] <- 0L
    a
}
print(r1())
# ... and an error after the rebind keeps the rebinding
r2 <- function() {
    a <- list(b = 1:3)
    try(a$b[{ a <- "kept"; stop("boom") }] <- 0L, silent = TRUE)
    a
}
print(r2())
# an error in an index leaves the target intact
r3 <- function() {
    a <- list(b = list(c = 1:3))
    try(a$b$c[{ stop("no") }] <- 0L, silent = TRUE)
    try(a[[{ stop("no") }]]$c <- 0L, silent = TRUE)
    a
}
print(r3())
m <- function() {
    a <- list(m = matrix(1:4, 2))
    i <- 1
    a$m[i, ] <- 0L
    a$m[, { print(a$m[1, 1]); 2 }] <- 9L
    a
}
print(m())
d <- function(...) {
    a <- list(b = 1:3)
    a$b[...] <- 0L
    a
}
print(d(2))
p <- function(i) {
    a <- list(b = 1:3)
    a$b[i] <- 0L
    a
}
print(p({ cat("forced\n"); 3 }))
# a rebind by an earlier argument stays for later ones
s1 <- function() {
    a <- list(b = 1:3)
    try(a[[{ a <- "kept"; "b" }]][{ print(a); stop("boom") }] <- 0L, silent = TRUE)
    a
}
print(s1())
# a rebind while a getter's arguments are evaluated again: the assignment wins
s2 <- function() {
    a <- list(b = list(c = 1))
    n <- 0L
    a[[{ n <- n + 1L; if (n == 2L) a <- "gone"; "b" }]]$c <- 2
    a
}
print(s2())
# ... and so does a rebind by R code the setter ran
s3 <- function() {
    a <- list(b = 1:3)
    n <- 0L
    as.character.foo <- function(x, ...) { a <<- "kept"; "z" }
    try(names(a[[{
        n <- n + 1L
        if (n == 2L) { print(a); stop("boom") }
        "b"
    }]]) <- structure("z", class = "foo"), silent = TRUE)
    print(a)
}
s3()
