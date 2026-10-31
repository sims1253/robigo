# Warnings raised by .Last and the exit finalizers are printed after the
# finalizers (R: unix/sys-std.c:Rstd_CleanUp's PrintWarnings).
.Last <- function() warning("warn-last")
reg.finalizer(new.env(), function(e) warning("warn-fin"), onexit = TRUE)
cat("ok\n")
