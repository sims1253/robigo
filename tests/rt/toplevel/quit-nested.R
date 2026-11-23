# quit() again from .Last: a second cut back to the top level; the first
# cut-off records stay reachable (R_ExitContext) and traced.
.Last <- function() {
    .Last <<- function() print(returnValue())
    quit("no", 7)
}
f <- function() { on.exit(quit("no", 3)); list(123) }
f()
