# An error in .Last under catch.script.errors (main.c:end_Rmainloop,
# check_session_exit): the session goes back to the console, which reads
# the end of the file, and .Last runs again. This one removes itself, so
# the second R_CleanUp finds no .Last and the session ends with status 0.
options(catch.script.errors = TRUE)
.Last <- function() {
    cat("in .Last\n")
    .Last <<- NULL
    stop("once")
}
cat("end of the script\n")
