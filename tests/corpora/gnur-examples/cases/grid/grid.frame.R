RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: grid.frame
### Title: Create a Frame for Packing Objects
### Aliases: grid.frame frameGrob
### Keywords: dplot

### ** Examples

grid.newpage()
grid.frame(name="gf", draw=TRUE)
grid.pack("gf", rectGrob(gp=gpar(fill="grey")), width=unit(1, "null"))
grid.pack("gf", textGrob("hi there"), side="right")



