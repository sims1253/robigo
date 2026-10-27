### Name: BOD
### Title: Biochemical Oxygen Demand
### Aliases: BOD
### Keywords: datasets

### ** Examples

## Don't show: 
options(show.nls.convergence=FALSE)
old <- options(digits = 5)
## End(Don't show)
require(stats)
# simplest form of fitting a first-order model to these data
fm1 <- nls(demand ~ A*(1-exp(-exp(lrc)*Time)), data = BOD,
           start = c(A = 20, lrc = log(.35)))
coef(fm1)
fm1
## IGNORE_RDIFF_BEGIN
# using the plinear algorithm  (trace o/p differs by platform)
fm2 <- nls(demand ~ (1-exp(-exp(lrc)*Time)), data = BOD,
           start = c(lrc = log(.35)), algorithm = "plinear", trace = TRUE)
## IGNORE_RDIFF_END
# using a self-starting model
fm3 <- nls(demand ~ SSasympOrig(Time, A, lrc), data = BOD)
summary(fm3)
## Don't show: 
options(old)
## End(Don't show)



