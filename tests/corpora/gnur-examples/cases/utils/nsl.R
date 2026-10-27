RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: nsl
### Title: Look up the IP Address by Hostname (on Unix-alikes)
### Aliases: nsl
### Keywords: utilities

### ** Examples

if(.Platform$OS.type == "unix") # includes Mac
  print( nsl("www.r-project.org") )



