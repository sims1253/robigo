RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: profile.nls
### Title: Method for Profiling 'nls' Objects
### Aliases: profile.nls
### Keywords: nonlinear regression models

### ** Examples

## Don't show: 
od <- options(digits = 4)
## End(Don't show)
# obtain the fitted object
fm1 <- nls(demand ~ SSasympOrig(Time, A, lrc), data = BOD)
# get the profile for the fitted model: default level is too extreme
pr1 <- profile(fm1, alphamax = 0.05)
# profiled values for the two parameters
## IGNORE_RDIFF_BEGIN
pr1$A
pr1$lrc
## IGNORE_RDIFF_END
# see also example(plot.profile.nls)
## Don't show: 
options(od)
## End(Don't show)



