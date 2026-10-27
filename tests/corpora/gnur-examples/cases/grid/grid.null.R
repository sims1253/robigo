RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: grid.null
### Title: Null Graphical Object
### Aliases: grid.null nullGrob
### Keywords: dplot

### ** Examples

grid.newpage()
grid.null(name="ref")
grid.rect(height=grobHeight("ref"))
grid.segments(0, 0, grobX("ref", 0), grobY("ref", 0))



