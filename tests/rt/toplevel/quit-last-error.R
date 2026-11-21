# An error in .Last() inside quit(): reported, then R halts (status 1,
# not quit()'s), the exit finalizers run, and nothing after quit() runs.
invisible(reg.finalizer(.BaseNamespaceEnv, function(e) cat("exit finalizer\n"), onexit = TRUE))
.Last <- function() stop("from .Last")
quit("no", 5)
cat("after\n")
