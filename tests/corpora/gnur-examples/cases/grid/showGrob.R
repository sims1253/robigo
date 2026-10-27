library(grid)
### Name: showGrob
### Title: Label grid grobs
### Aliases: showGrob
### Keywords: dplot

### ** Examples

    grid.newpage()
    gt <- gTree(childrenvp=vpStack(
                  viewport(x=0, width=.5, just="left", name="vp"),
                  viewport(y=.5, height=.5, just="bottom", name="vp2")),
                children=gList(rectGrob(vp="vp::vp2", name="child")),
                name="parent")
    grid.draw(gt)
    showGrob()
    showGrob(gPath="child")
    showGrob(recurse=FALSE)
    showGrob(depth=1)
    showGrob(depth=2)
    showGrob(depth=1:2)
    showGrob(gt)
    showGrob(gt, gPath="child")
    showGrob(just="left", gp=gpar(col="red", cex=.5), rot=45)
    showGrob(labelfun=function(grob, ...) {
        x <- grobX(grob, "west")
        y <- grobY(grob, "north")
        gTree(children=gList(rectGrob(x=x, y=y,
                width=stringWidth(grob$name) + unit(2, "mm"),
                height=stringHeight(grob$name) + unit(2, "mm"),
                gp=gpar(col=NA, fill=rgb(1, 0, 0, .5)),
                just=c("left", "top")),
                textGrob(grob$name,
                         x=x + unit(1, "mm"), y=y - unit(1, "mm"),
                         just=c("left", "top"))))
    })

## Not run: 
##D     # Examples from higher-level packages
##D 
##D     library(lattice)
##D     # Ctrl-c after first example
##D     example(histogram)
##D     showGrob()
##D     showGrob(gPath="plot_01.ylab")
##D 
##D     library(ggplot2)
##D     # Ctrl-c after first example
##D     example(qplot)
##D     showGrob()
##D     showGrob(recurse=FALSE)
##D     showGrob(gPath="panel-3-3")
##D     showGrob(gPath="axis.title", grep=TRUE)
##D     showGrob(depth=2)
## End(Not run)



