RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: depth
### Title: Determine the number of levels in an object
### Aliases: depth depth.viewport depth.path
### Keywords: dplot

### ** Examples

vp <- viewport()
depth(vp)
depth(vpStack(vp, vp))
depth(vpList(vpStack(vp, vp), vp))
depth(vpPath("vp"))
depth(vpPath("vp1", "vp2"))



