RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: constrOptim
### Title: Linearly Constrained Optimization
### Aliases: constrOptim
### Keywords: optimize

### ** Examples
## No test: 
##D ## from optim
##D fr <- function(x) {   ## Rosenbrock Banana function
##D     x1 <- x[1]
##D     x2 <- x[2]
##D     100 * (x2 - x1 * x1)^2 + (1 - x1)^2
##D }
##D grr <- function(x) { ## Gradient of 'fr'
##D     x1 <- x[1]
##D     x2 <- x[2]
##D     c(-400 * x1 * (x2 - x1 * x1) - 2 * (1 - x1),
##D        200 *      (x2 - x1 * x1))
##D }
##D 
##D optim(c(-1.2,1), fr, grr)
##D #Box-constraint, optimum on the boundary
##D constrOptim(c(-1.2,0.9), fr, grr, ui = rbind(c(-1,0), c(0,-1)), ci = c(-1,-1))
##D #  x <= 0.9,  y - x > 0.1
##D constrOptim(c(.5,0), fr, grr, ui = rbind(c(-1,0), c(1,-1)), ci = c(-0.9,0.1))
##D 
##D 
##D ## Solves linear and quadratic programming problems
##D ## but needs a feasible starting value
##D #
##D # from example(solve.QP) in 'quadprog'
##D # no derivative
##D fQP <- function(b) {-sum(c(0,5,0)*b)+0.5*sum(b*b)}
##D Amat       <- matrix(c(-4,-3,0,2,1,0,0,-2,1), 3, 3)
##D bvec       <- c(-8, 2, 0)
##D constrOptim(c(2,-1,-1), fQP, NULL, ui = t(Amat), ci = bvec)
##D # derivative
##D gQP <- function(b) {-c(0, 5, 0) + b}
##D constrOptim(c(2,-1,-1), fQP, gQP, ui = t(Amat), ci = bvec)
##D 
##D ## Now with maximisation instead of minimisation
##D hQP <- function(b) {sum(c(0,5,0)*b)-0.5*sum(b*b)}
##D constrOptim(c(2,-1,-1), hQP, NULL, ui = t(Amat), ci = bvec,
##D             control = list(fnscale = -1))
## End(No test)


