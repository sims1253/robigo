# Active bindings: assignment calls fun(base::quote(value)) and a read calls
# fun(), both evaluated in the global environment (envir.c setActiveValue,
# getActiveValue).
err <- function(expr) tryCatch(expr, error = function(e) print(conditionMessage(e)))
makeActiveBinding("y", function() 1, environment())
err(y <- 2)
err(y <- quote(a + b))
e <- new.env()
makeActiveBinding("y", function() 1, e)
err(assign("y", 3:6, envir = e))
err(e$y <- 4)
err(local(y <<- 5, new.env(parent = e)))
makeActiveBinding("z", function(v) {
  if (missing(v)) return("read")
  print(sys.call())
  print(v)
  invisible()
}, environment())
z <- 3
z <- quote(x)
z <- "s"
z
f <- function() {
  makeActiveBinding("w", function(v) {
    cat("global parent:", identical(parent.frame(), globalenv()), "\n")
    1
  }, environment())
  w
  w <- 2
  invisible()
}
f()
