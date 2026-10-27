RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: savehistory
### Title: Load or Save or Display the Commands History
### Aliases: loadhistory savehistory history timestamp
### Keywords: utilities

### ** Examples
## Not run: 
##D ## Save the history in the home directory: note that it is not
##D ## (by default) read from there but from the current directory
##D .Last <- function()
##D     if(interactive()) try(savehistory("~/.Rhistory"))
## End(Not run)


