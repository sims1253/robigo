# Native loop features that came from different PRs, used together.

# length() of a number (#317: an integer or a double depending on the
# path) and of a string element.
f <- function() {
    x <- 0L
    z <- 0L
    for (i in 1:3000) {
        if (i %% 2L == 0L) x <- 1L else x <- 2
        z <- length(x)
    }
    c(z, x)
}
print(f())
g <- function(ch) {
    z <- 0L
    for (i in 1:3000) z <- z + length(ch[[1L]])
    z
}
print(g(c("a", "b")))

# A callee that `<<-`-assigns a variable (a cell, #312) and another that
# reads it as a free variable beside an element read (#325).
k <- 0
w <- c(0, 0)
up <- function() k <<- k + 1
rd <- function() k + w[[1L]]
run <- function() {
    total <- 0
    for (i in 1:3000) {
        up()
        total <- total + rd()
    }
    total
}
print(run())
print(k)

# `x[[i]] <<- v` (#327) while an argument still holds the old vector.
x <- c(1, 2)
run2 <- function() {
    ans <- 0
    for (i in 1:3000) {
        if (i == 2000L) ans <- sum(x, { x[[1L]] <<- 10; 0 })
    }
    ans
}
print(run2())
print(x)
