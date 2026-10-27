RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: roundrect
### Title: Draw a rectangle with rounded corners
### Aliases: roundrect grid.roundrect roundrectGrob
### Keywords: dplot

### ** Examples

grid.roundrect(width=.5, height=.5, name="rr")
theta <- seq(0, 360, length.out=50)
for (i in 1:50)
    grid.circle(x=grobX("rr", theta[i]),
                y=grobY("rr", theta[i]),
                r=unit(1, "mm"),
                gp=gpar(fill="black"))



