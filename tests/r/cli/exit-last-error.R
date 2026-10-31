# An error in .Last is reported at top level, then the finalizers run and
# the session fails with status 1 (R: main.c:R_dot_Last).
.Last <- function() { cat("last\n"); stop("last failed") }
reg.finalizer(new.env(), function(e) cat("finalizer\n"), onexit = TRUE)
cat("ok\n")
