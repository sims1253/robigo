# Closure calls from compiled code: promises with bytecode, lookup memo
f <- function(x) x
g <- function() { f(return("early")); "late" }
g()
lazy <- function() { n <- 1; h <- function(a) { n <<- 100; a }; h(n - 1) }
lazy()
unused <- function(a) "never forced"
k <- function() unused(stop("not evaluated"))
k()
m <- function(a, b) c(missing(a), missing(b))
w <- function(x) m(x)
w()
w(1)
s <- function(a) substitute(a)
s2 <- function(q) s(q + 1)
s2(5)
sc <- function(...) sys.call()
mc <- function(alpha, beta) match.call()
caller <- function(v) list(sc(v * 2), mc(v + 1, b = v))
caller(3)
id <- function(x) x
vis <- function() id(invisible(7))
vis()
print(vis())
withVisible(id(invisible(1)))$visible
err <- function(x) x + 1
tc <- function() tryCatch(err("a"), error = function(e) conditionMessage(e))
tc()
we <- function() withCallingHandlers(err(log(-1)), warning = function(w) { cat("warn:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
we()
dots <- function(...) list(...)
fwd <- function(...) dots(..., z = 3)
str1 <- fwd(1, y = 2)
names(str1)
unlist(str1)
cnt <- function(n) if (n == 0) 0 else 1 + cnt(n - 1)
cnt(200)
loopcall <- function() {
  out <- numeric(0)
  for (i in 1:4) {
    if (i == 3) sq <- function(x) -x
    out <- c(out, sq(i))
  }
  out
}
sq <- function(x) x * x
loopcall()
shadow <- function() { sq <- function(x) 0; sq(5) }
shadow()
sq(5)
redefine <- function() {
  r <- numeric(0)
  for (i in 1:3) {
    r <- c(r, sq(i))
    sq <<- function(x) x + 1000
  }
  r
}
redefine()
sq(1)
deep <- function(n) deep(n + 1)
res <- tryCatch(deep(1), error = function(e) "too deep")
res
argfun <- function(f) f(2)
argfun(function(z) z * 10)
compose <- function(n) { add <- function(a) a + n; add(n * 2) }
compose(4)
# guards in loops see functions rebound mid-loop: locally, in an enclosing
# closure's frame, and globally
local_rebind <- function() {
  out <- numeric(0)
  for (i in 1:3) {
    out <- c(out, length(c(i, i)))
    if (i == 1) length <- function(x) -1
  }
  out
}
local_rebind()
outer <- function() {
  sqrt <- base::sqrt
  inner <- function() {
    r <- numeric(0)
    for (i in 1:3) {
      r <- c(r, sqrt(i * i))
      if (i == 2) sqrt <<- function(x) -x
    }
    r
  }
  inner()
}
outer()
abs <- base::abs
glob <- function() {
  r <- numeric(0)
  for (i in 1:3) {
    r <- c(r, abs(-i))
    if (i == 1) abs <<- function(x) 0
  }
  r
}
glob()
rm(abs)
abs(-2)
