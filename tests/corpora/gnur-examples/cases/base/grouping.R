RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: grouping
### Title: Grouping Permutation
### Aliases: grouping
### Keywords: manip

### ** Examples

(ii <- grouping(x <- c(1, 1, 3:1, 1:4, 3), y <- c(9, 9:1), z <- c(2, 1:9)))
## 6  5  2  1  7  4 10  8  3  9
rbind(x, y, z)[, ii]



