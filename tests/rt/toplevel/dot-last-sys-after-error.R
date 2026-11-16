# An error in .Last() ends the session: .Last.sys() does not run after it
# (no context of its own; check_session_exit's R_CleanUp runs no .Last).
.Last <- function() stop("from .Last")
.Last.sys <- function() cat("a global .Last.sys\n")
cat("body\n")
