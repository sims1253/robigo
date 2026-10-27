RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: PP.test
### Title: Phillips-Perron Test for Unit Roots
### Aliases: PP.test
### Keywords: ts

### ** Examples

x <- rnorm(1000)
PP.test(x)
y <- cumsum(x) # has unit root
PP.test(y)



