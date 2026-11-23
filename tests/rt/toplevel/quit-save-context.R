# The save without .Last first: sys.save.image sees quit()'s callers.
sys.save.image <- function(file) print(sys.calls())
f <- function() quit("yes", 4, FALSE)
f()
