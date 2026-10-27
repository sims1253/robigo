RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: getWindowsHandles
### Title: Get handles of Windows in the MS Windows RGui
### Aliases: getWindowsHandles
### Keywords: utilities

### ** Examples
## No test: 
##D if(.Platform$OS.type == "windows") withAutoprint({
##D   getWindowsHandles()
##D   getWindowsHandles("all")
##D })
## End(No test)


