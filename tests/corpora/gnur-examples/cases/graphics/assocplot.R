### Name: assocplot
### Title: Association Plots
### Aliases: assocplot
### Keywords: hplot

### ** Examples

## Aggregate over sex:
x <- marginSums(HairEyeColor, c(1, 2))
x
assocplot(x, main = "Relation between hair and eye color")



