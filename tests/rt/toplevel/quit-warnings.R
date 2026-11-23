# The warnings pending when quit() ends R are printed (R_CleanUp's
# PrintWarnings), with .Last's.
.Last <- function() warning("from .Last")
f <- function() { warning("from f"); quit("no", 1) }
f()
