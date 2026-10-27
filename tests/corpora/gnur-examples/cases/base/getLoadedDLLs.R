RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: getLoadedDLLs
### Title: Get DLLs Loaded in Current Session
### Aliases: getLoadedDLLs print.DLLInfo print.DLLInfoList [.DLLInfoList
###   $.DLLInfo DLLInfo DLLInfoList
### Keywords: interface

### ** Examples

getLoadedDLLs()

utils::tail(getLoadedDLLs(), 2) # the last 2 loaded ones, still a DLLInfoList



