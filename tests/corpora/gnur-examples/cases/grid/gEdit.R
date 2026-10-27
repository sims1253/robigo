RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: gEdit
### Title: Create and Apply Edit Objects
### Aliases: gEdit gEditList applyEdit applyEdits
### Keywords: dplot

### ** Examples

grid.rect(gp=gpar(col="red"))
# same thing, but more verbose
grid.draw(applyEdit(rectGrob(), gEdit(gp=gpar(col="red"))))



