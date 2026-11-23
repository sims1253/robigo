# An NA runLast: a warning, and FALSE (.Last does not run).
.Last <- function() cat("in .Last\n")
quit("no", 1, NA)
