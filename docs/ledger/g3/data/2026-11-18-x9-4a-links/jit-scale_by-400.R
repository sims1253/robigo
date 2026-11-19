unlockBinding(".AutoloadEnv", baseenv()); assign(".AutoloadEnv", new.env(), envir = baseenv()); bindtextdomain <- function(...) NULL
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
g <- function() { for (i in 1:400) { if (typeof(.Internal(bodyCode(compiler:::tryCmpfun(scale_by)))) != "bytecode") stop("left") }; NULL }
g()
