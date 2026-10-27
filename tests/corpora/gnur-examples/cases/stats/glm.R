RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: glm
### Title: Fitting Generalized Linear Models
### Aliases: glm glm.fit weights.glm
### Keywords: models regression

### ** Examples

## Dobson (1990) Page 93: Randomized Controlled Trial :
counts <- c(18,17,15,20,10,20,25,13,12)
outcome <- gl(3,1,9)
treatment <- gl(3,3)
data.frame(treatment, outcome, counts) # showing data
glm.D93 <- glm(counts ~ outcome + treatment, family = poisson())
anova(glm.D93)
## No test: 
##D summary(glm.D93)
## End(No test)
## Computing AIC [in many ways]:
(A0 <- AIC(glm.D93))
(ll <- logLik(glm.D93))
A1 <- -2*c(ll) + 2*attr(ll, "df")
A2 <- glm.D93$family$aic(counts, mu=fitted(glm.D93), wt=1) +
        2 * length(coef(glm.D93))
stopifnot(exprs = {
  all.equal(A0, A1)
  all.equal(A1, A2)
  all.equal(A1, glm.D93$aic)
})


## No test: 
##D ## an example with offsets from Venables & Ripley (2002, p.189)
##D utils::data(anorexia, package = "MASS")
##D 
##D anorex.1 <- glm(Postwt ~ Prewt + Treat + offset(Prewt),
##D                 family = gaussian, data = anorexia)
##D summary(anorex.1)
## End(No test)

# A Gamma example, from McCullagh & Nelder (1989, pp. 300-2)
clotting <- data.frame(
    u = c(5,10,15,20,30,40,60,80,100),
    lot1 = c(118,58,42,35,27,25,21,19,18),
    lot2 = c(69,35,26,21,18,16,13,12,12))
summary(glm(lot1 ~ log(u), data = clotting, family = Gamma))
summary(glm(lot2 ~ log(u), data = clotting, family = Gamma))
## Aliased ("S"ingular) -> 1 NA coefficient
(fS <- glm(lot2 ~ log(u) + log(u^2), data = clotting, family = Gamma))
tools::assertError(update(fS, singular.ok=FALSE), verbose=interactive())
## -> .. "singular fit encountered"

## Not run: 
##D ## for an example of the use of a terms object as a formula
##D demo(glm.vr)
## End(Not run)


