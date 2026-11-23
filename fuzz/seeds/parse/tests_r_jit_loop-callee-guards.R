# Native loops calling closures inline check the functions those closures
# use (and their own) on every entry: rebinding one between runs, or in the
# loop's frame between two loops, takes effect.
s <- 0L
gen <- function() {
    s <<- s + 1L
    s * 2L
}
run <- function(n) {
    x <- integer(n)
    for (i in 1:n) x[[i]] <- gen()
    sum(x)
}
print(run(100))
`+` <- function(e1, e2) base::`-`(e1, e2)
print(run(100))
print(s)
rm(`+`)
print(run(100))
print(s)
`*` <- function(e1, e2) 0L
print(run(50))
rm(`*`)
print(run(50))

# In the loop's frame: the second loop sees a local `+`.
twice <- function(n) {
    v <- 0
    for (i in 1:n) v <- v + i
    `+` <- function(e1, e2) base::`*`(e1, e2)
    w <- 1
    for (i in 1:n) w <- w + 1
    c(v, w)
}
print(twice(30))
print(twice(30))

# A new frame each call, some calls binding the function locally.
maybe <- function(n, local) {
    if (local) `-` <- function(e1, e2) 99
    t <- 0
    for (i in 1:n) t <- t - 1
    t
}
for (l in c(FALSE, TRUE, FALSE, TRUE)) print(maybe(40, l))

# The callee's own environment rebinding the function.
mk <- function() {
    k <- 0L
    step <- function() k <<- k + 1L
    list(step = step, get = function() k, env = environment())
}
o <- mk()
loop <- function(f, n) for (i in 1:n) f()
loop(o$step, 60)
print(o$get())
assign("+", function(e1, e2) base::`+`(e1, 10L), envir = o$env)
loop(o$step, 60)
print(o$get())
