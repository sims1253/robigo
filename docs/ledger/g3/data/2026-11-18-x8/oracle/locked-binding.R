# X8 premise (2026-11-18-x8-premise.md): GNU R keeps the old vector intact
# when code runs or an error is raised after EnlargeVector.
f <- function() {
  x <- as.numeric(1:3); lockBinding("x", environment())
  r <- tryCatch({ x[5] <- 9; "ok" }, error = function(e) conditionMessage(e))
  cat("locked:", r, "| x =", x, "| length", length(x), "\n")
}
f()
