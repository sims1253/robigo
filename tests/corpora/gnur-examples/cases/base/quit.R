### Name: quit
### Title: Terminate an R Session
### Aliases: quit q .Last .Last.sys
### Keywords: environment

### ** Examples

## Not run: 
##D ## Unix-flavour example
##D .Last <- function() {
##D   graphics.off() # close devices before printing
##D   cat("Now sending PDF graphics to the printer:\n")
##D   system("lpr Rplots.pdf")
##D   cat("bye bye...\n")
##D }
##D quit("yes")
## End(Not run)



