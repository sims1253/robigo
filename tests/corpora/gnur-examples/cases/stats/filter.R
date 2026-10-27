RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: filter
### Title: Linear Filtering on a Time Series
### Aliases: filter
### Keywords: ts

### ** Examples

x <- 1:100
filter(x, rep(1, 3))
filter(x, rep(1, 3), sides = 1)
filter(x, rep(1, 3), sides = 1, circular = TRUE)

filter(presidents, rep(1, 3))



