# X8 premise (2026-11-18-x8-premise.md): GNU R keeps the old vector intact
# when code runs or an error is raised after EnlargeVector.
k <- function() {
  l <- list(1, 2)
  withCallingHandlers({ l[3:4] <- list(7, 8, 9) },
    warning = function(w) { cat("list handler sees length(l) =", length(l), "\n"); invokeRestart("muffleWarning") })
  cat("after:", length(l), "\n")
}
k()
