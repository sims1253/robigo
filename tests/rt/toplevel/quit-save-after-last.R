# After .Last the context is the top level: an exiting handler around
# quit() finds no function to return to, and R halts.
sys.save.image <- function(file) stop("save")
tryCatch(quit("yes", 0, TRUE), error = function(e) cat("caught\n"))
cat("after\n")
