withCallingHandlers(
  { warning("w1"); cat("after w1\n"); "value" },
  warning = function(w) {
    cat("handler saw:", conditionMessage(w), "\n")
    invokeRestart("muffleWarning")
  }
)
withCallingHandlers(
  { message_like <- simpleCondition("msg"); class(message_like) <- c("custom", "condition"); signalCondition(message_like); "done" },
  custom = function(c) cat("custom condition\n")
)
f <- function() {
  withCallingHandlers(stop("deep"), error = function(e) cat("calling handler for:", conditionMessage(e), "\n"))
}
tryCatch(f(), error = function(e) cat("then exiting handler\n"))
