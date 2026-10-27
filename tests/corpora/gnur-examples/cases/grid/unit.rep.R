RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: unit.rep
### Title: Replicate Elements of Unit Objects
### Aliases: unit.rep
### Keywords: dplot

### ** Examples

rep(unit(1:3, "npc"), 3)
rep(unit(1:3, "npc"), 1:3)
rep(unit(1:3, "npc") + unit(1, "inches"), 3)
rep(max(unit(1:3, "npc") + unit(1, "inches")), 3)
rep(max(unit(1:3, "npc") + unit(1, "strwidth", "a"))*4, 3)
rep(unit(1:3, "npc") + unit(1, "strwidth", "a")*4, 3)



