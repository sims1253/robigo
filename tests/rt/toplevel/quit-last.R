# quit() runs .Last first (runLast = TRUE, the default).
.Last <- function() cat("in .Last\n")
quit("no", 2)
