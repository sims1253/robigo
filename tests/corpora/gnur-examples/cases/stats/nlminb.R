### Name: nlminb
### Title: Optimization using PORT routines
### Aliases: nlminb
### Keywords: optimize

### ** Examples
## No test: 
##D x <- rnbinom(100, mu = 10, size = 10)
##D hdev <- function(par)
##D     -sum(dnbinom(x, mu = par[1], size = par[2], log = TRUE))
##D nlminb(c(9, 12), hdev)
##D nlminb(c(20, 20), hdev, lower = 0, upper = Inf)
##D nlminb(c(20, 20), hdev, lower = 0.001, upper = Inf)
##D 
##D ## slightly modified from the S-PLUS help page for nlminb
##D # this example minimizes a sum of squares with known solution y
##D sumsq <- function( x, y) {sum((x-y)^2)}
##D y <- rep(1,5)
##D x0 <- rnorm(length(y))
##D nlminb(start = x0, sumsq, y = y)
##D # now use bounds with a y that has some components outside the bounds
##D y <- c( 0, 2, 0, -2, 0)
##D nlminb(start = x0, sumsq, lower = -1, upper = 1, y = y)
##D # try using the gradient
##D sumsq.g <- function(x, y) 2*(x-y)
##D nlminb(start = x0, sumsq, sumsq.g,
##D        lower = -1, upper = 1, y = y)
##D # now use the hessian, too
##D sumsq.h <- function(x, y) diag(2, nrow = length(x))
##D nlminb(start = x0, sumsq, sumsq.g, sumsq.h,
##D        lower = -1, upper = 1, y = y)
##D 
##D ## Rest lifted from optim help page
##D 
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
##D nlminb(c(-1.2,1), fr)
##D nlminb(c(-1.2,1), fr, grr)
##D 
##D 
##D flb <- function(x)
##D     { p <- length(x); sum(c(1, rep(4, p-1)) * (x - c(1, x[-p])^2)^2) }
##D ## 25-dimensional box constrained
##D ## par[24] is *not* at boundary
##D nlminb(rep(3, 25), flb, lower = rep(2, 25), upper = rep(4, 25))
##D ## trying to use a too small tolerance:
##D r <- nlminb(rep(3, 25), flb, control = list(rel.tol = 1e-16))
##D stopifnot(grepl("rel.tol", r$message))
## End(No test)


