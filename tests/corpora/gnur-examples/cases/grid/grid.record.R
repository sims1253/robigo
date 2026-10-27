RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: grid.record
### Title: Encapsulate calculations and drawing
### Aliases: grid.record recordGrob
### Keywords: dplot

### ** Examples

grid.record({
              w <- convertWidth(unit(1, "inches"), "npc")
              grid.rect(width=w)
            },
            list())



