### Name: alias
### Title: Find Aliases (Dependencies) in a Model
### Aliases: alias alias.formula alias.lm
### Keywords: models

### ** Examples
## No test: ##D 
##D op <- options(contrasts = c("contr.helmert", "contr.poly"))
##D npk.aov <- aov(yield ~ block + N*P*K, npk)
##D alias(npk.aov)
##D options(op)  # reset
## End(No test)


