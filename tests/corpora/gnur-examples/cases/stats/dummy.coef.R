RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: dummy.coef
### Title: Extract Coefficients in Original Coding
### Aliases: dummy.coef dummy.coef.lm dummy.coef.aovlist
### Keywords: models

### ** Examples

options(contrasts = c("contr.helmert", "contr.poly"))
## From Venables and Ripley (2002) p.165.
npk.aov <- aov(yield ~ block + N*P*K, npk)
dummy.coef(npk.aov)

npk.aovE <- aov(yield ~  N*P*K + Error(block), npk)
dummy.coef(npk.aovE)



