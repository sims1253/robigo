RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: any
### Title: Are Some Values True?
### Aliases: any
### Keywords: logic

### ** Examples

range(x <- sort(round(stats::rnorm(10) - 1.2, 1)))
if(any(x < 0)) cat("x contains negative values\n")



