# R code a replacement runs (warnings, calling handlers) sees the target's
# current value, not a temporarily emptied binding (#527).
h <- function(w) { print(x); invokeRestart("muffleWarning") }
x <- structure("bad", class = "foo")
withCallingHandlers(class(x) <- "integer", warning = h)
print(x)
x <- "bad"
withCallingHandlers(class(x) <- "integer", warning = h)
print(x)
x <- "bad"
withCallingHandlers(storage.mode(x) <- "integer", warning = h)
print(x)
x <- structure("bad", class = "foo")
withCallingHandlers(storage.mode(x) <- "integer", warning = h)
print(x)
x <- structure("bad", class = "foo")
withCallingHandlers(class(x) <- "numeric", warning = h)
print(x)
x <- structure("bad", class = "foo")
tryCatch(withCallingHandlers(class(x) <- "integer", warning = function(w) stop("boom")), error = function(e) cat("caught\n"))
print(x)
x <- 1:6
withCallingHandlers(dim(x) <- c(2+1i, 3), warning = h)
print(x)
x <- structure(1:6, class = "foo")
withCallingHandlers(class(x) <- "integer", warning = h)
print(x)
# in a function (VM)
f <- function() {
  x <- structure("bad", class = "foo")
  withCallingHandlers(class(x) <- "integer", warning = function(w) { print(x); invokeRestart("muffleWarning") })
  y <- "bad"
  withCallingHandlers(storage.mode(y) <- "integer", warning = function(w) { print(y); invokeRestart("muffleWarning") })
  z <- 1:6
  withCallingHandlers(dim(z) <- c(2+1i, 3), warning = function(w) { print(z); invokeRestart("muffleWarning") })
  list(x, y, z)
}
print(f())
