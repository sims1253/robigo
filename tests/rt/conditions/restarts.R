r <- withRestarts(
  { cat("before\n"); invokeRestart("myRestart", 1, 2); cat("not reached\n") },
  myRestart = function(a, b) a + b
)
r
x <- withRestarts(stop("e1"), abort = function() "aborted")
x
f <- function() {
  withRestarts(
    { warning("w"); "normal" },
    muffleWarning = function() "muffled"
  )
}
withCallingHandlers(f(), warning = function(w) invokeRestart("muffleWarning"))
computeRestarts()[[1]]
