RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: arrangeWindows
### Title: Rearrange Windows on MS Windows
### Aliases: arrangeWindows
### Keywords: utilities

### ** Examples

## Not run: 
##D ## Only available on Windows :
##D arrangeWindows("v")
##D # This default is useful only in SDI mode:  it will tile any Firefox window
##D # along with the R windows
##D .arrangeWindowsDefaults <- list(c("R", "all"), pattern = c("", "Firefox"))
##D arrangeWindows("v")
## End(Not run)



