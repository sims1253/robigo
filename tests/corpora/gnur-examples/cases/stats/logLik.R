### Name: logLik
### Title: Extract Log-Likelihood
### Aliases: logLik logLik.lm
### Keywords: models

### ** Examples

x <- 1:5
lmx <- lm(x ~ 1)
logLik(lmx) # using print.logLik() method
utils::str(logLik(lmx))

## lm method
(fm1 <- lm(rating ~ ., data = attitude))
logLik(fm1)
logLik(fm1, REML = TRUE)

## No test: 
##D utils::data(Orthodont, package = "nlme")
##D fm1 <- lm(distance ~ Sex * age, Orthodont)
##D logLik(fm1)
##D logLik(fm1, REML = TRUE)
## End(No test)


