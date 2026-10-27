RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: HairEyeColor
### Title: Hair and Eye Color of Statistics Students
### Aliases: HairEyeColor
### Keywords: datasets

### ** Examples

require(graphics)
## Full mosaic
mosaicplot(HairEyeColor)
## Aggregate over sex (as in Snee's original data)
x <- apply(HairEyeColor, c(1, 2), sum)
x
mosaicplot(x, main = "Relation between hair and eye color")



