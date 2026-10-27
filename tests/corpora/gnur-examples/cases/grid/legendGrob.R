RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: legendGrob
### Title: Constructing a Legend Grob
### Aliases: legendGrob grid.legend
### Keywords: aplot

### ** Examples

  ## Data:
  n <- 10
  x <- stats::runif(n) ; y1 <- stats::runif(n) ; y2 <- stats::runif(n)
  ## Construct the grobs :
  plot <- gTree(children=gList(rectGrob(),
                  pointsGrob(x, y1, pch=21, gp=gpar(col=2, fill="gray")),
                  pointsGrob(x, y2, pch=22, gp=gpar(col=3, fill="gray")),
                  xaxisGrob(),
                  yaxisGrob()))
  legd <- legendGrob(c("Girls", "Boys", "Other"), pch=21:23,
                     gp=gpar(col = 2:4, fill = "gray"))
  gg <- packGrob(packGrob(frameGrob(), plot),
                 legd, height=unit(1,"null"), side="right")

  ## Now draw it on a new device page:
  grid.newpage()
  pushViewport(viewport(width=0.8, height=0.8))
  grid.draw(gg)



