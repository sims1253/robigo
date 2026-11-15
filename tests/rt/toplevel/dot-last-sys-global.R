# R_dot_Last calls .Last.sys() by name from R_BaseEnv: one bound only in the
# global environment is found by findVar (through the base namespace's
# parent) but not by the call, so the session ends with that error.
.Last.sys <- function() cat("a global .Last.sys\n")
cat("body\n")
