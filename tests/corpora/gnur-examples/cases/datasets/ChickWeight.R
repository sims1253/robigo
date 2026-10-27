RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: ChickWeight
### Title: Weight versus age of chicks on different diets
### Aliases: ChickWeight
### Keywords: datasets

### ** Examples

require(graphics)
coplot(weight ~ Time | Chick, data = ChickWeight,
       type = "b", show.given = FALSE)



