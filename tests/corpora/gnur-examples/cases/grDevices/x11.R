RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: x11
### Title: X Window System Graphics (X11)
### Aliases: x11 X11 X11.options
### Keywords: device

### ** Examples
## Not run: 
##D if(.Platform$OS.type == "unix") { # Only on unix-alikes, possibly Mac,
##D ## put something like this is your .Rprofile to customize the defaults
##D setHook(packageEvent("grDevices", "onLoad"),
##D         function(...) grDevices::X11.options(width = 8, height = 6, xpos = 0,
##D                                              pointsize = 10))
##D }
## End(Not run)


