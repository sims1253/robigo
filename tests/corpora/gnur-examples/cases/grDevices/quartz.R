### Name: quartz
### Title: macOS Quartz Device
### Aliases: quartz quartz.options quartz.save
### Keywords: device

### ** Examples
## Not run: 
##D ## Only on a Mac,
##D ## put something like this is your .Rprofile to customize the defaults
##D setHook(packageEvent("grDevices", "onLoad"),
##D         function(...) grDevices::quartz.options(width = 8, height = 6,
##D                                                 pointsize = 10))
## End(Not run)


