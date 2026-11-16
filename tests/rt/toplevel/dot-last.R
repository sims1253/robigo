# R_CleanUp at the end of a session (main.c:R_dot_Last, unix/sys-std.c):
# .Last() runs, then the exit finalizers.
invisible(reg.finalizer(.BaseNamespaceEnv, function(e) cat("exit finalizer\n"), onexit = TRUE))
.Last <- function() cat("in .Last\n")
cat("body\n")
