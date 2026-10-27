RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: DNase
### Title: Elisa assay of DNase
### Aliases: DNase
### Keywords: datasets

### ** Examples

require(stats); require(graphics)
## Don't show: 
options(show.nls.convergence=FALSE)
## End(Don't show)
coplot(density ~ conc | Run, data = DNase,
       show.given = FALSE, type = "b")
coplot(density ~ log(conc) | Run, data = DNase,
       show.given = FALSE, type = "b")
## fit a representative run
fm1 <- nls(density ~ SSlogis( log(conc), Asym, xmid, scal ),
    data = DNase, subset = Run == 1)
## compare with a four-parameter logistic
fm2 <- nls(density ~ SSfpl( log(conc), A, B, xmid, scal ),
    data = DNase, subset = Run == 1)
summary(fm2)
anova(fm1, fm2)



