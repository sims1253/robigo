RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: getWindowsHandle
### Title: Get a Windows Handle
### Aliases: getWindowsHandle
### Keywords: utilities

### ** Examples

if(.Platform$OS.type == "windows")
  print( getWindowsHandle() )



