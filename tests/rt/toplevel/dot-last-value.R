# .Last()'s value is not .Last.value: an exit finalizer still sees the last
# top-level value (R_dot_Last stores it in R_CurrentExpr).
invisible(reg.finalizer(.BaseNamespaceEnv, function(e) print(.Last.value), onexit = TRUE))
.Last <- function() 42
invisible(7)
