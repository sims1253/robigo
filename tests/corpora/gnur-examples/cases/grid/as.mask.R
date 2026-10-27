RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: as.mask
### Title: Define a Soft Mask
### Aliases: as.mask
### Keywords: dplot

### ** Examples

## NOTE: on devices without support for masks normal line segments 
##       will be drawn
grid.newpage()
## Alpha mask
grid.segments(y0=1, y1=0, gp=gpar(col=2, lwd=100))
pushViewport(viewport(mask=circleGrob(gp=gpar(fill=rgb(0,0,0,.5)))))
grid.segments(gp=gpar(col=3, lwd=100))
grid.newpage()
## Luminance mask
grid.segments(y0=1, y1=0, gp=gpar(col=2, lwd=100))
pushViewport(viewport(mask=as.mask(circleGrob(gp=gpar(fill="grey50")),
                                   "luminance")))
grid.segments(gp=gpar(col=3, lwd=100))



