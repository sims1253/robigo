# .Last runs with the context cut back to the top level (R_dot_Last):
# no frame of the function that called quit().
.Last <- function() { print(sys.calls()); print(sys.nframe()) }
f <- function() quit("no", 2)
f()
