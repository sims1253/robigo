# R: unix/sys-std.c:Rstd_CleanUp -- .Last runs on a normal exit, then the
# onexit finalizers (reg.finalizer(..., onexit = TRUE)).
.Last <- function() cat("last\n")
reg.finalizer(new.env(), function(e) cat("finalizer\n"), onexit = TRUE)
cat("ok\n")
