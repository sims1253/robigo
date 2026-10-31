# An error in the script skips .Last but still runs the onexit finalizers
# (R: unix/sys-std.c:Rstd_CleanUp).
.Last <- function() cat("last\n")
reg.finalizer(new.env(), function(e) cat("finalizer\n"), onexit = TRUE)
stop("boom")
