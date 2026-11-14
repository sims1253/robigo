options(warn = 1)
f <- function() { warning("printed at once"); cat("after the warning\n"); 1 }
f()
options(warn = 0)
f()
cat("deferred above\n")
options(warn = 2)
g <- function() { warning("turned into an error"); cat("not reached\n") }
g()
