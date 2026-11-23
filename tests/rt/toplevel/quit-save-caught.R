# runLast = FALSE: the save runs in quit()'s context, so its error is
# quit()'s, and a handler around quit() catches it.
sys.save.image <- function(file) stop("save")
tryCatch(quit("yes", 0, FALSE), error = function(e) cat("caught\n"))
cat("after\n")
