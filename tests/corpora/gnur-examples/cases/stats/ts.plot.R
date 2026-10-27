RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: ts.plot
### Title: Plot Multiple Time Series
### Aliases: ts.plot
### Keywords: ts

### ** Examples

require(graphics)

ts.plot(ldeaths, mdeaths, fdeaths,
        gpars=list(xlab="year", ylab="deaths", lty=c(1:3)))



