RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: grid.delay
### Title: Encapsulate calculations and generating a grob
### Aliases: grid.delay delayGrob
### Keywords: dplot

### ** Examples

grid.delay({
              w <- convertWidth(unit(1, "inches"), "npc")
              rectGrob(width=w)
            },
            list())



