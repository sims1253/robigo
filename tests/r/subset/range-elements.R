# Elements of a variable holding an integer sequence made by `:` (which
# compiled code keeps unmaterialized), read with `[[` and `[` in closures
# and loops.
f <- function(n) {
  perm1 <- 1:n
  out <- integer(0)
  for (it in 1:6) {
    if (k <- perm1[[1L]]) out <- c(out, k)
    perm1[c(1, 2)] <- perm1[c(2, 1)]
  }
  out
}
print(f(4))
g <- function(n) { x <- 1:n; c(x[[1]], x[[n]], x[2], length(x[[2]]), x[[2L]] + 0.5) }
print(g(5))
h <- function(a, b) { x <- a:b; c(x[[1]], x[[length(x)]], x[[2]]) }
print(h(10, 3)); print(h(-2, 2))
z <- function() { x <- 1:3; tryCatch(x[[4]], error = function(e) conditionMessage(e)) }
print(z())
w <- function() { x <- 3:1; x[[2]] <- 10L; x }
print(w())
