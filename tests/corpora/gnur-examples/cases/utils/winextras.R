RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: winextras
### Title: Get Windows Version
### Aliases: win.version
### Keywords: utilities

### ** Examples

if(.Platform$OS.type == "windows")
   print(win.version())



