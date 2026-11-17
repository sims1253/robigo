# main.c:check_session_exit: with options(error) set or
# options(catch.script.errors) TRUE, a script goes on after an error that
# reaches the top level; the rest of the failing line is dropped (the
# console's buffer starts over).
options(catch.script.errors = TRUE)
f <- function() stop("first")
f(); cat("not reached: the rest of the line\n")
cat("the next line runs\n")
stop("at the top level")
cat("still going\n")
x <- )
cat("after a syntax error\n")
options(catch.script.errors = FALSE)
options(error = quote(cat("the error option ran\n")))
g <- function() stop("second")
g()
cat("after the error option\n")
options(error = NULL)
warning("deferred")
h <- function() stop("final")
h()
cat("not reached: the session halts\n")
