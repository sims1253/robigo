RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: diffinv
### Title: Discrete Integration: Inverse of Differencing
### Aliases: diffinv diffinv.default diffinv.ts
### Keywords: ts

### ** Examples

s <- 1:10
d <- diff(s)
diffinv(d, xi = 1)



