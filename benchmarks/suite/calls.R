# Closure calls that take the general path: neither inlined nor native
# code (defaults left to their promise, named arguments, a string argument,
# a function found in an environment, `<<-` state, S3 dispatch, tryCatch).
# The costs of a frame, promises and the VM's entry, which the suite's other
# programs (pure scalar trees) no longer pay.
n <- 12000
scale_by <- function(x, scale = 1, label = "a") { if (label == "zz") x else x * scale }
weigh <- function(x, y) { z <- x * y; z + 1 }
tag <- function(x, label) { if (label == "bc") x * 2 else x }
e <- new.env()
e$f <- function(x) x + 1
acct <- function() {
  bal <- 0
  list(dep = function(x) { bal <<- bal + x; invisible(bal) }, get = function() bal)
}
a <- acct()
area <- function(s) UseMethod("area")
area.sq <- function(s) s$side^2
area.default <- function(s) 0
sq <- structure(list(side = 2), class = "sq")
safe <- function(x) tryCatch(if (x > 0) x else stop("neg"), error = function(err) 0)
s <- 0
for (i in 1:n) {
  s <- s + scale_by(i * 0.5) + weigh(y = 2, x = i) + tag(i * 0.25, "bc") + e$f(i)
  a$dep(i %% 7)
  s <- s + area(sq) + safe(i - 6000)
}
cat(s, a$get(), "\n")
