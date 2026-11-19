# X8 premise (2026-11-18-x8-premise.md): GNU R keeps the old vector intact
# when code runs or an error is raised after EnlargeVector.
g <- function() {
  x <- as.numeric(1:3)
  op <- options(warn = 2); on.exit(options(op))
  r <- tryCatch({ x[4:5] <- c(7, 8, 9); "ok" }, error = function(e) conditionMessage(e))
  cat("warn2:", r, "| x =", x, "\n")
}
g()
