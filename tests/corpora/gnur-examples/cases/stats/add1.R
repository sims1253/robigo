RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: add1
### Title: Add or Drop All Possible Single Terms to a Model
### Aliases: add1 add1.default add1.lm add1.glm drop1 drop1.default
###   drop1.lm drop1.glm
### Keywords: models

### ** Examples

## Don't show: 
od <- options(digits = 5)
## End(Don't show)
require(graphics); require(utils)
## following example(swiss)
lm1 <- lm(Fertility ~ ., data = swiss)
add1(lm1, ~ I(Education^2) + .^2)
drop1(lm1, test = "F")  # So called 'type II' anova

## following example(glm)
## Don't show: 
example(glm, echo = FALSE)
## End(Don't show)
drop1(glm.D93, test = "Chisq")
drop1(glm.D93, test = "F")
add1(glm.D93, scope = ~outcome*treatment, test = "Rao") ## Pearson Chi-square
## Don't show: 
options(od)
## End(Don't show)



