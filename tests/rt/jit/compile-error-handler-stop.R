# tryCmpfun (#964), nothing wider: a calling handler established outside
# the compile runs with the handlers below its own, so its stop() for a
# warning of the compile is not tryCmpfun's and leaves the call (the
# closure never runs). Run under LC_ALL=C.
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
n <- 0L
h <- function(w) { n <<- n + 1L; if (n == 1L) stop("from the handler") }
r <- try(withCallingHandlers(f(), warning = h), silent = TRUE)
print(r)
print(n)
