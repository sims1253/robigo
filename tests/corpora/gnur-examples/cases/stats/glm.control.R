RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: glm.control
### Title: Auxiliary for Controlling GLM Fitting
### Aliases: glm.control
### Keywords: optimize models

### ** Examples
## No test: 
##D ### A variation on  example(glm) :
##D 
##D ## Annette Dobson's example ...
##D counts <- c(18,17,15,20,10,20,25,13,12)
##D outcome <- gl(3,1,9)
##D treatment <- gl(3,3)
##D oo <- options(digits = 12) # to see more when tracing :
##D glm.D93X <- glm(counts ~ outcome + treatment, family = poisson(),
##D                 trace = TRUE, epsilon = 1e-14)
##D options(oo)
##D coef(glm.D93X) # the last two are closer to 0 than in ?glm's  glm.D93
## End(No test)


