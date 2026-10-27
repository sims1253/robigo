### Name: windows.options
### Title: Auxiliary Function to Set/View Defaults for Arguments of
###   windows()
### Aliases: windows.options
### Keywords: device

### ** Examples
## Not run: 
##D ## put something like this is your .Rprofile to customize the defaults
##D setHook(packageEvent("grDevices", "onLoad"),
##D         function(...)
##D             grDevices::windows.options(width = 8, height = 6,
##D                                        xpos = 0, pointsize = 10,
##D                                        bitmap.aa.win = "cleartype"))
## End(Not run)


