# Native loops calling a helper closure made anew by each call of the
# function around the loop: the cells are the bindings of the instance
# called now, not of the one the loop was compiled with.
counter <- function(n, k0) {
    k <- k0
    tick <- function() k <<- k + 1L
    for (i in 1:n) tick()
    k
}
print(counter(1500L, 0L))
print(counter(1500L, 10L))
print(counter(2000L, -5L))

scaled <- function(n, f) {
    add <- function(i) f * i
    s <- 0
    for (i in 1:n) s <- s + add(i)
    s
}
print(scaled(1000L, 1))
print(scaled(1000L, 100))
print(scaled(1000L, 0.5))
