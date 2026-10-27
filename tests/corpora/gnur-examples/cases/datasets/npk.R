### Name: npk
### Title: Classical N, P, K Factorial Experiment
### Aliases: npk
### Keywords: datasets

### ** Examples
## No test: 
##D options(contrasts = c("contr.sum", "contr.poly"))
##D npk.aov <- aov(yield ~ block + N*P*K, npk)
##D npk.aov
##D summary(npk.aov)
##D coef(npk.aov)
##D options(contrasts = c("contr.treatment", "contr.poly"))
##D npk.aov1 <- aov(yield ~ block + N + K, data = npk)
##D summary.lm(npk.aov1)
##D se.contrast(npk.aov1, list(N=="0", N=="1"), data = npk)
##D model.tables(npk.aov1, type = "means", se = TRUE)
## End(No test)


