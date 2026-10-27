RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: Querying the Viewport Tree
### Title: Get the Current Grid Viewport (Tree)
### Aliases: current.viewport current.parent current.vpTree current.vpPath
###   current.transform current.rotation
### Keywords: dplot

### ** Examples

grid.newpage()
pushViewport(viewport(width=0.8, height=0.8, name="A"))
pushViewport(viewport(x=0.1, width=0.3, height=0.6,
  just="left", name="B"))
upViewport(1)
pushViewport(viewport(x=0.5, width=0.4, height=0.8,
  just="left", name="C"))
pushViewport(viewport(width=0.8, height=0.8, name="D"))
current.vpPath()
upViewport(1)
current.vpPath()
current.vpTree()
current.viewport()
current.vpTree(all=FALSE)
popViewport(0)



