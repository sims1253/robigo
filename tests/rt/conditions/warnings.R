f <- function() { warning("first"); warning("second"); 10 }
f()
g <- function(x) { if (x < 0) warning("negative input"); sqrt(abs(x)) }
g(-4)
cat("between\n")
h <- function() { for (i in 1:3) warning(paste("w", i)); invisible() }
h()
r <- withCallingHandlers(f(), warning = function(w) invokeRestart("muffleWarning"))
r
k <- function() warning("a long warning message that should be long enough to wrap the line")
k()
