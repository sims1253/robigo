# errors.c:do_addGlobHands, the internal of globalCallingHandlers() (whose
# R code needs utils::removeSource): global calling handlers live on the
# top level's handler stack, below every handler a call establishes.
h <- function(w) {
    cat("global handler:", conditionMessage(w), "\n")
    invokeRestart("muffleWarning")
}
print(.Internal(.addGlobHands("warning", list(h), .GlobalEnv, NULL, TRUE)))
warning("w1")
f <- function() warning("w2 in f")
f()
g <- function() withCallingHandlers(warning("w3 in g"), warning = function(w) cat("local handler first\n"))
g()
print(tryCatch(local(.Internal(.addGlobHands("warning", list(h), .GlobalEnv, NULL, TRUE))),
               error = conditionMessage))
k <- function() .Internal(.addGlobHands("warning", list(h), .GlobalEnv, NULL, TRUE))
k()
warning("w4 after a call that set them")
m <- function(e) cat("global error handler:", conditionMessage(e), "\n")
.Internal(.addGlobHands(c("warning", "error"), list(h, m), .GlobalEnv, NULL, TRUE))
print(tryCatch(stop("e1"), error = function(e) "tryCatch wins"))
.Internal(.addGlobHands(NULL, NULL, .GlobalEnv, NULL, TRUE))
warning("w5 deferred: no global handlers left")
print(tryCatch(.Internal(.addGlobHands("warning", 1, .GlobalEnv, NULL, TRUE)), error = conditionMessage))
