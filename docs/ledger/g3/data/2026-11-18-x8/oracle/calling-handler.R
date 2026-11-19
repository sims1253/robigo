# X8 premise (2026-11-18-x8-premise.md): GNU R keeps the old vector intact
# when code runs or an error is raised after EnlargeVector.
h <- function() {
  x <- as.numeric(1:3)
  withCallingHandlers({ x[4:5] <- c(7, 8, 9) },
    warning = function(w) { cat("handler sees length(x) =", length(x), "x =", x, "\n"); invokeRestart("muffleWarning") })
  cat("after:", x, "\n")
}
h()
