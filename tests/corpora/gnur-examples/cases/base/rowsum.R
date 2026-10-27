RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: rowsum
### Title: Give Column Sums of a Matrix or Data Frame, Based on a Grouping
###   Variable
### Aliases: rowsum rowsum.default rowsum.data.frame
### Keywords: manip

### ** Examples

require(stats)

x <- matrix(runif(100), ncol = 5)
group <- sample(1:8, 20, TRUE)
(xsum <- rowsum(x, group))
## Slower versions
tapply(x, list(group[row(x)], col(x)), sum)
t(sapply(split(as.data.frame(x), group), colSums))
aggregate(x, list(group), sum)[-1]



