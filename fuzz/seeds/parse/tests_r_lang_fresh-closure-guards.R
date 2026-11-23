# Closures made anew in each call of a function (the `function(x) ...` passed
# to lapply) skip their guards once those are known to hold for frames of
# that function. A frame that binds a guarded name, or a rebinding between
# calls, must still be seen.
f <- function(v) { g <- function(x) length(x) + 1L; g(v) }
for (k in 1:3) print(f(1:k))
h <- function(v, shadow) {
  if (shadow) length <- function(x) 99L
  g <- function(x) length(x) + 1L
  g(v)
}
print(h(1:2, FALSE)); print(h(1:2, TRUE)); print(h(1:2, FALSE)); print(h(1:2, TRUE))
p <- function(v) { g <- function(x) is.numeric(x) && length(x) > 1; c(g(v), g(v[1])) }
print(p(1:3)); print(p(1:3))
length <- function(x) -1L
print(f(1:3)); print(p(1:3))
rm(length)
print(f(1:3)); print(p(1:3))
q <- function(v) {
  g <- function(x) length(x)
  r1 <- g(v)
  length <- function(x) 42L
  c(r1, g(v))
}
print(q(1:5)); print(q(1:5))
s <- function(v) vapply(v, function(x) is.numeric(x) || is.logical(x), NA)
print(s(list(1, "a", TRUE))); print(s(list(1, "a", TRUE)))
lst <- function(n) lapply(seq_len(n), function(i) if (i > 1) i * 2 else -i)
print(lst(3)); print(lst(2))
w <- function(len) { g <- function(x) length(x); g(1:4) + len }
print(w(1)); print(w(2))
m <- function(length) { g <- function(x) length(x); g(1:4) }
print(m(function(x) 7L)); print(tryCatch(m(3), error = conditionMessage))
order(c(3, 1, 2)); print(order(c(3, 1, 2), c(1, 1, 1))); print(sort.list(c(2, 1)))
