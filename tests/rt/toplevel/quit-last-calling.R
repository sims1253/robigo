# A calling handler around quit() still runs for .Last's warning (the
# handler stack is kept); the warning is printed at the end.
.Last <- function() warning("W")
withCallingHandlers(quit("no", 3), warning = function(w) cat("handler\n"))
