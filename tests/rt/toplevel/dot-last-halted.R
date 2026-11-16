# A script that halts on an error runs no .Last(), but the exit finalizers.
invisible(reg.finalizer(.BaseNamespaceEnv, function(e) cat("exit finalizer\n"), onexit = TRUE))
.Last <- function() cat("in .Last\n")
stop("from the script")
