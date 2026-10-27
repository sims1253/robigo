RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: anova.glm
### Title: Analysis of Deviance for Generalized Linear Model Fits
### Aliases: anova.glm
### Keywords: models regression

### ** Examples

## --- Continuing the Example from  '?glm':
## Don't show: 
require(utils)
example("glm", echo = FALSE)
## End(Don't show)
anova(glm.D93, test = FALSE)
anova(glm.D93, test = "Cp")
anova(glm.D93, test = "Chisq")
glm.D93a <-
   update(glm.D93, ~treatment*outcome) # equivalent to Pearson Chi-square
anova(glm.D93, glm.D93a, test = "Rao")



