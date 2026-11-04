# A native loop entered many times checks its guards and calls once while
# nothing they depend on changes, and again when something does.
h <- function(a, b) a * 2 + b
f <- function(n, m) {
  s <- 0
  for (k in 1:n) {
    for (j in 1:m) s <- s + h(j, k)
    if (k == 3) cat("k", k, "s", s, "\n")
  }
  s
}
for (i in 1:3) print(f(6, 20))
h <- function(a, b) a - b          # rebound globally: the next entry sees it
print(f(6, 20))
g <- function(n, m) {
  s <- 0
  for (k in 1:n) {
    if (k == 4) `+` <- function(a, b) base::`+`(a, b) * 2   # in this frame
    for (j in 1:m) s <- s + j
    cat("k", k, "s", s, "\n")
  }
  s
}
for (i in 1:2) print(g(6, 20))
mk <- function(w) { hw <- function(a, b) a * w + b; function(n, m) { s <- 0; for (k in 1:n) for (j in 1:m) s <- s + hw(j, k); s } }
p <- mk(3)
for (i in 1:3) print(p(5, 20))
environment(p)$w <- 10
print(p(5, 20))
q <- mk(2)
print(q(5, 20))
# The helper's operators come from its own environment, which can change
# between entries without anything else changing.
e <- new.env()
assign("+", base::`+`, envir = e)
hh <- function(x) x + 1
environment(hh) <- e
lockEnvironment(e, bindings = FALSE)
ff <- function(n) {
  tot <- 0
  for (r in 1:5) {
    if (r == 4) assign("+", function(a, b) base::`+`(a, base::`*`(b, 10)), envir = e)
    s <- 0
    for (k in 1:n) s <- s + hh(k)
    tot <- tot + s
    cat("r", r, "s", s, "\n")
  }
  tot
}
print(ff(30))
# The helper is rebound in an enclosing frame (not the loop's own) between
# entries.
pr <- function(n) {
  hq <- function(a) a + 1
  g2 <- function() {
    tot <- 0
    for (r in 1:5) {
      if (r == 3) hq <<- function(a) a * 100
      s <- 0
      for (j in 1:n) s <- s + hq(j)
      tot <- tot + s
      cat("r", r, "s", s, "\n")
    }
    tot
  }
  g2()
}
print(pr(30)); print(pr(30))
