library(grid)
### Name: grid.stroke
### Title: Stroke or Fill a Path
### Aliases: grid.stroke strokeGrob strokeGrob.grob strokeGrob.GridPath
###   grid.fill fillGrob fillGrob.grob fillGrob.GridPath grid.fillStroke
###   fillStrokeGrob fillStrokeGrob.grob fillStrokeGrob.GridPath as.path
### Keywords: dplot

### ** Examples

## NOTE: on devices without support for stroking and filling
##       nothing will be drawn
grid.newpage()
grid.stroke(textGrob("hello", gp=gpar(cex=10)))
grid.fill(circleGrob(1:2/3, r=.3), gp=gpar(fill=rgb(1,0,0,.5)))



