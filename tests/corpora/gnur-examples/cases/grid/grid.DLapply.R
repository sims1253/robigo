RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: grid.DLapply
### Title: Modify the Grid Display List
### Aliases: grid.DLapply
### Keywords: dplot

### ** Examples

grid.newpage()
grid.rect(width=.4, height=.4, x=.25, y=.75, gp=gpar(fill="black"), name="r1")
grid.rect(width=.4, height=.4, x=.5, y=.5, gp=gpar(fill="grey"), name="r2")
grid.rect(width=.4, height=.4, x=.75, y=.25, gp=gpar(fill="white"), name="r3")
grid.DLapply(function(x) { if (is.grob(x)) x$gp <- gpar(); x })
grid.refresh()



