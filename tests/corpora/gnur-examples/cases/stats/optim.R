### Name: optim
### Title: General-purpose Optimization
### Aliases: optim optimHess
### Keywords: nonlinear optimize

### ** Examples
## No test: 
##D require(graphics)
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
##D optim(c(-1.2,1), fr)
##D (res <- optim(c(-1.2,1), fr, grr, method = "BFGS"))
##D optimHess(res$par, fr, grr)
##D optim(c(-1.2,1), fr, NULL, method = "BFGS", hessian = TRUE)
##D ## These do not converge in the default number of steps
##D optim(c(-1.2,1), fr, grr, method = "CG")
##D optim(c(-1.2,1), fr, grr, method = "CG", control = list(type = 2))
##D optim(c(-1.2,1), fr, grr, method = "L-BFGS-B")
##D 
##D flb <- function(x)
##D     { p <- length(x); sum(c(1, rep(4, p-1)) * (x - c(1, x[-p])^2)^2) }
##D ## 25-dimensional box constrained
##D optim(rep(3, 25), flb, NULL, method = "L-BFGS-B",
##D       lower = rep(2, 25), upper = rep(4, 25)) # par[24] is *not* at boundary
##D 
##D 
##D ## "wild" function , global minimum at about -15.81515
##D fw <- function (x)
##D     10*sin(0.3*x)*sin(1.3*x^2) + 0.00001*x^4 + 0.2*x+80
##D plot(fw, -50, 50, n = 1000, main = "optim() minimising 'wild function'")
##D 
##D res <- optim(50, fw, method = "SANN",
##D              control = list(maxit = 20000, temp = 20, parscale = 20))
##D res
##D ## Now improve locally {typically only by a small bit}:
##D (r2 <- optim(res$par, fw, method = "BFGS"))
##D points(r2$par,  r2$value,  pch = 8, col = "red", cex = 2)
##D 
##D ## Combinatorial optimization: Traveling salesman problem
##D library(stats) # normally loaded
##D 
##D eurodistmat <- as.matrix(eurodist)
##D 
##D distance <- function(sq) {  # Target function
##D     sq2 <- embed(sq, 2)
##D     sum(eurodistmat[cbind(sq2[,2], sq2[,1])])
##D }
##D 
##D genseq <- function(sq) {  # Generate new candidate sequence
##D     idx <- seq(2, NROW(eurodistmat)-1)
##D     changepoints <- sample(idx, size = 2, replace = FALSE)
##D     tmp <- sq[changepoints[1]]
##D     sq[changepoints[1]] <- sq[changepoints[2]]
##D     sq[changepoints[2]] <- tmp
##D     sq
##D }
##D 
##D sq <- c(1:nrow(eurodistmat), 1)  # Initial sequence: alphabetic
##D distance(sq)
##D # rotate for conventional orientation
##D loc <- -cmdscale(eurodist, add = TRUE)$points
##D x <- loc[,1]; y <- loc[,2]
##D s <- seq_len(nrow(eurodistmat))
##D tspinit <- loc[sq,]
##D 
##D plot(x, y, type = "n", asp = 1, xlab = "", ylab = "",
##D      main = "initial solution of traveling salesman problem", axes = FALSE)
##D arrows(tspinit[s,1], tspinit[s,2], tspinit[s+1,1], tspinit[s+1,2],
##D        angle = 10, col = "green")
##D text(x, y, labels(eurodist), cex = 0.8)
##D 
##D set.seed(123) # chosen to get a good soln relatively quickly
##D res <- optim(sq, distance, genseq, method = "SANN",
##D              control = list(maxit = 30000, temp = 2000, trace = TRUE,
##D                             REPORT = 500))
##D res  # Near optimum distance around 12842
##D 
##D tspres <- loc[res$par,]
##D plot(x, y, type = "n", asp = 1, xlab = "", ylab = "",
##D      main = "optim() 'solving' traveling salesman problem", axes = FALSE)
##D arrows(tspres[s,1], tspres[s,2], tspres[s+1,1], tspres[s+1,2],
##D        angle = 10, col = "red")
##D text(x, y, labels(eurodist), cex = 0.8)
##D 
##D ## 1-D minimization: "Brent" or optimize() being preferred.. but NM may be ok and "unavoidable",
##D ## ----------------   so we can suppress the check+warning :
##D system.time(rO <- optimize(function(x) (x-pi)^2, c(0, 10)))
##D system.time(ro <- optim(1, function(x) (x-pi)^2, control=list(warn.1d.NelderMead = FALSE)))
##D rO$minimum - pi # 0 (perfect), on one platform
##D ro$par - pi     # ~= 1.9e-4    on one platform
##D utils::str(ro)
## End(No test)


