### Name: windows
### Title: Windows Graphics Devices
### Aliases: windows win.graph win.metafile win.print print.SavedPlots
###   [.SavedPlots
### Keywords: device

### ** Examples
## Not run: 
##D ## A series of plots written to a sequence of metafiles
##D if(.Platform$OS.type == "windows")
##D    win.metafile("Rplot%02d.wmf", pointsize = 10)
## End(Not run)


