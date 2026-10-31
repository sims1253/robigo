# q(runLast = FALSE) skips .Last but still runs the onexit finalizers.
.Last <- function() cat("last\n")
reg.finalizer(new.env(), function(e) cat("finalizer\n"), onexit = TRUE)
q("no", 3, FALSE)
