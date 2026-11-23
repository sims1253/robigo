# Each top-level statement stores its value in .Last.value, visible or
# not.
1 + 1
.Last.value
x <- 5
.Last.value
stopifnot(identical(.Last.value, 5))
"fin"
