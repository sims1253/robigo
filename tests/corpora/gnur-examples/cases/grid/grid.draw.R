RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: grid.draw
### Title: Draw a grid grob
### Aliases: grid.draw
### Keywords: dplot

### ** Examples

grid.newpage()
## Create a graphical object, but don't draw it
l <- linesGrob()
## Draw it
grid.draw(l)



