RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: summary.lm
### Title: Summarizing Linear Model Fits
### Aliases: summary.lm summary.mlm print.summary.lm
### Keywords: regression models

### ** Examples

## Don't show: 
utils::example("lm", echo = FALSE)
## End(Don't show)
##-- Continuing the  lm(.) example:
coef(lm.D90)  # the bare coefficients
sld90 <- summary(lm.D90 <- lm(weight ~ group -1))  # omitting intercept
sld90
coef(sld90)  # much more

## model with *aliased* coefficient:
lm.D9. <- lm(weight ~ group + I(group != "Ctl"))
Sm.D9. <- summary(lm.D9.)
Sm.D9. #  shows the NA NA NA NA  line
stopifnot(length(cc <- coef(lm.D9.)) == 3, is.na(cc[3]),
          dim(coef(Sm.D9.)) == c(2,4), Sm.D9.$df == c(2, 18, 3))



