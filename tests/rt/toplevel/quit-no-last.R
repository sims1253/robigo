# runLast = FALSE: .Last does not run.
.Last <- function() cat("in .Last\n")
quit("no", 2, FALSE)
