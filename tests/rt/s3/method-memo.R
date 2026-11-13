# S3 method lookup memo (g3.md §9.3): methods defined, shadowed, rebound and
# masked between dispatches, with the global environment and a frame that
# binds .packageName as the top environment.
area <- function(s) UseMethod("area")
area.default <- function(s) "default"
x <- structure(1, class = "sq")
f <- function() area(x)
f()
area.sq <- function(s) "sq global"
f()
g <- function() { area.sq <- function(s) "sq local"; area(x) }
g()
f()
area(x)
area.sq <- NULL
f()
area.sq <- 5
f()
area.sq <- function(s) "sq again"
f()
area.sq <- NULL
f()
area.default <- function(s) "default2"
f()
p <- structure(1, class = "foo")
print(p)
print.foo <- function(x, ...) cat("foo!\n")
print(p)
print.foo <- NULL
print(p)
h <- function() { print.foo <- function(x, ...) cat("local foo\n"); print(p) }
h()
print(p)
k <- (function() function() area(x))()
k()
area.sq <- function(s) "sq last"
k()
mk <- function() {
  .packageName <- "pkg"
  area.sq <- NULL
  list(call = function(o) area(o), set = function(f) area.sq <<- f)
}
m <- mk()
m$call(x)
m$set(function(s) "sq in pkg")
m$call(x)
m$set(NULL)
m$call(x)
area.sq <- function(s) "sq global final"
m$call(x)
area.sq <- NULL
m$call(x)
y <- structure(1, class = c("sq", "rect"))
area.rect <- function(s) "rect"
m$call(y)
f2 <- function() area(y)
f2()
area.sq <- function(s) "sq beats rect"
f2()
m$call(y)
