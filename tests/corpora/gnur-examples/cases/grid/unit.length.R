RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: unit.length
### Title: Length of a Unit Object
### Aliases: unit.length
### Keywords: dplot

### ** Examples

length(unit(1:3, "npc"))
length(unit(1:3, "npc") + unit(1, "inches"))
length(max(unit(1:3, "npc") + unit(1, "inches")))
length(max(unit(1:3, "npc") + unit(1, "strwidth", "a"))*4)
length(unit(1:3, "npc") + unit(1, "strwidth", "a")*4)



