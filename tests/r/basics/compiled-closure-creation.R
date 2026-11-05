## `function(...)` evaluated by compiled code (MAKECLOSURE): the closure
## sees its defining frame as it is when it runs, keeps formals, defaults,
## body and srcref, and works when made in loops, defaults and lapply.
f <- function() { x <- 1; g <- function() x; x <- 2; g() }
print(f())
adders <- lapply(1:3, function(i) function(x) x + i)
print(adders[[2]](10))
h <- function(a) { k <- function(b = a * 2) b; a <- 5; k() }
print(h(1))
cnt <- function() { n <- 0; function() { n <<- n + 1; n } }
cc <- cnt(); cc(); print(cc())
w <- function() { y <- 10; fun <- function(z) y + z; y <- 20; environment(fun)$y }
print(w())
loopc <- function() { r <- 0; for (i in 1:5) { f <- function() i * 2; r <- r + f() }; r }
print(loopc())
same_env <- function() { a <- function() 1; identical(environment(a), environment()) }
print(same_env())
shapes <- function() {
  fs <- list(function(x) x + 1, function(a, b = 2, ...) NULL, function() NULL)
  for (f in fs) { print(formals(f)); print(body(f)) }
}
shapes()
vis <- function() function() 1
withVisible(vis())$visible
mkmany <- function(n) { s <- 0; for (i in seq_len(n)) { g <- function(v) v * i; s <- s + g(2) }; s }
print(mkmany(100))
rebound <- function() { `function` <- function(...) "not a closure"; function() 1 }
print(rebound())
