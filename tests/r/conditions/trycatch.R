r <- tryCatch(stop("oops"), error = function(e) paste("caught:", conditionMessage(e)))
r
r2 <- tryCatch(warning("careful"), warning = function(w) paste("warned:", conditionMessage(w)))
r2
r3 <- tryCatch({
  10
}, error = function(e) -1, finally = cat("finally ran\n"))
r3
f <- function() stop("inner failure")
tryCatch(f(), error = function(e) print(conditionCall(e)))
res <- withCallingHandlers(
  {
    warning("w1")
    "value"
  },
  warning = function(w) {
    cat("handling:", conditionMessage(w), "\n")
    invokeRestart("muffleWarning")
  }
)
res
try(log(-1), silent = TRUE)
t <- try(stop("visible error"), silent = TRUE)
class(t)
cat(t)
inherits(t, "try-error")
cond <- simpleCondition("custom message")
class(cond) <- c("myCondition", "condition")
r4 <- withCallingHandlers(
  {
    signalCondition(cond)
    "after signal"
  },
  myCondition = function(c) cat("saw custom condition\n")
)
r4
r5 <- tryCatch(signalCondition(cond), condition = function(c) "caught custom")
r5
e <- simpleError("my error")
tryCatch(stop(e), error = function(err) conditionMessage(err))
suppressWarnings({
  warning("hidden")
  "done"
})
nested <- tryCatch({
  tryCatch(stop("deep"), error = function(e) stop(paste("rethrown:", conditionMessage(e))))
}, error = function(e) conditionMessage(e))
nested
