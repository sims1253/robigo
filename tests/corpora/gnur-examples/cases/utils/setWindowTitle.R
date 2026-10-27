RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: setWindowTitle
### Title: Set the Window Title or the Status Bar of the RGui in Windows
### Aliases: setWindowTitle getWindowTitle getIdentification setStatusBar
### Keywords: utilities

### ** Examples

if(.Platform$OS.type == "windows") withAutoprint({
## show the current working directory in the title, saving the old one
oldtitle <- setWindowTitle(getwd())
Sys.sleep(0.5)
## reset the title
setWindowTitle("")
Sys.sleep(0.5)
## restore the original title
setWindowTitle(title = oldtitle)
})



