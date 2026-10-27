RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: aperm
### Title: Array Transposition
### Aliases: aperm aperm.default aperm.table
### Keywords: array

### ** Examples

# interchange the first two subscripts on a 3-way array x
x  <- array(1:24, 2:4)
xt <- aperm(x, c(2,1,3))
stopifnot(t(xt[,,2]) == x[,,2],
          t(xt[,,3]) == x[,,3],
          t(xt[,,4]) == x[,,4])

UCB <- aperm(UCBAdmissions, c(2,1,3))
UCB[1,,]
summary(UCB) # UCB is still a contingency table
## Don't show: 
stopifnot(is.table(UCB))
## End(Don't show)



