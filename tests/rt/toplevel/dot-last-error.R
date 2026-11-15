# An error in .Last(): reported, then "Execution halted"; the exit
# finalizers still run (check_session_exit's R_CleanUp), and the status is 1.
invisible(reg.finalizer(.BaseNamespaceEnv, function(e) cat("exit finalizer\n"), onexit = TRUE))
.Last <- function() { cat("in .Last\n"); stop("from .Last") }
cat("body\n")
