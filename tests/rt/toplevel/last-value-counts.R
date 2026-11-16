# .Last.value holds the last top-level value without a count (names.c's
# DISABLE_REFCNT(R_LastvalueSymbol)), and a value nothing references gets
# one (main.c:Rf_ReplIteration). So a variable bound to .Last.value
# shares it: a write copies, and .Last.value keeps the old value, also
# when the change happens before the next top-level statement replaces it.
c(1, 2, 3)
{ z <- .Last.value; z[1] <- 5; print(.Last.value); print(z) }
c(a = 1, b = 2)
{ w <- .Last.value; names(w) <- c("x", "y"); print(.Last.value); print(w) }
invisible(list(1, 2))
{ l <- .Last.value; l[[1]] <- "a"; print(.Last.value[[1]]); print(l[[1]]) }
# a variable's value keeps its own count: .Last.value is not one more
x <- c(1, 2, 3)
{ y <- .Last.value; x[1] <- 5; print(y); print(x) }
