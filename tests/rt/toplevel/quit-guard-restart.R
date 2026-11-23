# .Last invokes a restart established around quit(): its frame is one the
# reset abandoned, so there is no function to return to, and R halts.
.Last <- function() invokeRestart("myR")
f <- function() withRestarts(quit("no", 6), myR = function() cat("restarted\n"))
f()
cat("after\n")
