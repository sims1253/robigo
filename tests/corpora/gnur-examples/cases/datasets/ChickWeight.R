### Name: ChickWeight
### Title: Weight versus age of chicks on different diets
### Aliases: ChickWeight
### Keywords: datasets

### ** Examples

require(graphics)
coplot(weight ~ Time | Chick, data = ChickWeight,
       type = "b", show.given = FALSE)



