RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: model.tables
### Title: Compute Tables of Results from an 'aov' Model Fit
### Aliases: model.tables model.tables.aov model.tables.aovlist
### Keywords: models

### ** Examples
## No test: 
##D options(contrasts = c("contr.helmert", "contr.treatment"))
##D npk.aov <- aov(yield ~ block + N*P*K, npk)
##D model.tables(npk.aov, "means", se = TRUE)
##D 
##D ## as a test, not particularly sensible statistically
##D npk.aovE <- aov(yield ~  N*P*K + Error(block), npk)
##D model.tables(npk.aovE, se = TRUE)
##D model.tables(npk.aovE, "means")
## End(No test)


