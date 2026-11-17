# A script goes on past a syntax error under catch.script.errors
# (main.c:check_session_exit). The parse error is raised at the top level
# and its jump lands there, which restores the top level's handler stack:
# the global calling handler sees the parse error, the next statement's
# error and the final "unexpected end of input".
options(catch.script.errors = TRUE)
m <- function(e) cat("global error handler:", conditionMessage(e), "\n")
invisible(.Internal(.addGlobHands("error", list(m), .GlobalEnv, NULL, TRUE)))
x <- )
stop("second")
cat("after the second\n")
f(
