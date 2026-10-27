RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: xyTable
### Title: Multiplicities of (x,y) Points, e.g., for a Sunflower Plot
### Aliases: xyTable
### Keywords: dplot

### ** Examples

xyTable(iris[, 3:4], digits = 6)

## If missing coordinates exist, they are also counted
iris2 <- iris[1:10, 3:4]
iris2[4, 2] <- NA
iris2[c(3, 5), ] <- NA
xyTable(iris2)

## Discretized uncorrelated Gaussian:
## Don't show: 
set.seed(1)
## End(Don't show)
xy <- data.frame(x = round(sort(stats::rnorm(100))), y = stats::rnorm(100))
xyTable(xy, digits = 1)



