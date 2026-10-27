RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: step
### Title: Choose a model by AIC in a Stepwise Algorithm
### Aliases: step
### Keywords: models

### ** Examples
## No test: 
##D ## following on from example(lm)
## Don't show: 
##D utils::example("lm", echo = FALSE)
## End(Don't show)
##D 
##D step(lm.D9)
##D 
##D summary(lm1 <- lm(Fertility ~ ., data = swiss))
##D slm1 <- step(lm1)
##D summary(slm1)
##D slm1$anova
## End(No test)


