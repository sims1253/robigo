# for (j in seq(length = n)): GNU R's seq.default result, its checks, and
# dispatch that may leave seq.default.
f <- function(n) { s <- 0L; for (j in seq(length = n)) s <- s + j; c(s, if (exists("j", inherits = FALSE)) j else NA) }
print(f(5L)); print(f(0L)); print(f(3)); print(f(2.5)); print(f(TRUE)); print(f(FALSE))
g <- function(n) { r <- integer(); for (j in seq(length.out = n)) r <- c(r, j); r }
print(g(4)); print(typeof(g(3.2)))
h <- function(n) { k <- 0; for (j in seq(len = n)) k <- k + 1; k }
print(h(7L))
for (bad in list(-1, NA, Inf, NA_integer_, NaN)) print(tryCatch(f(bad), error = function(e) conditionMessage(e)))
print(tryCatch(f(c(2, 3)), warning = function(w) conditionMessage(w)))
print(tryCatch(f("3"), error = function(e) conditionMessage(e)))
seq.numeric <- function(...) { cat("seq.numeric\n"); 1:2 }
print(f(5))
rm(seq.numeric)
seq.integer <- function(...) { cat("seq.integer\n"); 3:4 }
print(f(5L)); print(f(5))
rm(seq.integer)
local({
  seq.default <- function(...) { cat("local seq.default\n"); 9L }
  k <- function(n) { s <- 0; for (j in seq(length = n)) s <- s + j; s }
  print(k(4))
})
w <- function(n) { seq <- function(...) 100:101; s <- 0; for (j in seq(length = n)) s <- s + j; s }
print(w(3))
lazy <- function(n) { s <- 0; for (j in seq(length = n)) s <- s + j; s }
print(tryCatch(lazy(), error = function(e) conditionMessage(e)))
print(lazy({ cat("forced\n"); 3 }))
conv <- function(nx, ny) {
  z <- numeric(nx + ny - 1)
  for (i in seq(length = nx)) for (j in seq(length = ny)) z[[i + j - 1]] <- z[[i + j - 1]] + i * j
  sum(z)
}
print(conv(30, 40))
# Re-enter a warmed native region with new lengths and dispatch methods.
hot <- function(n) {
  s <- 0
  for (i in 1:600) for (j in seq(length = n)) s <- s + j
  s
}
print(hot(3L)); print(hot(2.5)); print(hot(TRUE)); print(hot(0L))
for (bad in list(-1, NA, Inf, NaN, c(2, 3))) {
  print(tryCatch(hot(bad), error = function(e) conditionMessage(e),
                 warning = function(w) conditionMessage(w)))
}
seq.numeric <- function(...) 7L
print(hot(3))
rm(seq.numeric)
print(hot(3))
# The inner length cannot be hoisted when the outer body changes it.
changing <- function() {
  s <- 0; n <- 0L
  for (i in 1:600) {
    n <- 3L - n
    for (j in seq(length = n)) s <- s + j
  }
  s
}
print(changing()); print(changing())
