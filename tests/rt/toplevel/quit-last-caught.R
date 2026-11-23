# An error in .Last: the exiting handler around quit() is not reachable
# from the top level ("no function to return from"), and R halts.
.Last <- function() stop("L")
tryCatch(quit("no", 3), error = function(e) cat("caught\n"))
cat("after\n")
