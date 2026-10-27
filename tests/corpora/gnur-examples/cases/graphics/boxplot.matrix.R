RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: boxplot.matrix
### Title: Draw a Boxplot for each Column (Row) of a Matrix
### Aliases: boxplot.matrix
### Keywords: hplot

### ** Examples

## Very similar to the example in ?boxplot
mat <- cbind(Uni05 = (1:100)/21, Norm = rnorm(100),
             T5 = rt(100, df = 5), Gam2 = rgamma(100, shape = 2))
boxplot(mat, main = "boxplot.matrix(...., main = ...)",
        notch = TRUE, col = 1:4)



