### Name: smoothScatter
### Title: Scatterplots with Smoothed Densities Color Representation
### Aliases: smoothScatter
### Keywords: hplot

### ** Examples
## No test: 
##D ## A largish data set
##D n <- 10000
##D x1  <- matrix(rnorm(n), ncol = 2)
##D x2  <- matrix(rnorm(n, mean = 3, sd = 1.5), ncol = 2)
##D x   <- rbind(x1, x2)
##D 
##D oldpar <- par(mfrow = c(2, 2), mar=.1+c(3,3,1,1), mgp = c(1.5, 0.5, 0))
##D smoothScatter(x, nrpoints = 0)
##D smoothScatter(x)
##D 
##D ## a different color scheme:
##D Lab.palette <- colorRampPalette(c("blue", "orange", "red"), space = "Lab")
##D i.s <- smoothScatter(x, colramp = Lab.palette,
##D                      ## pch=NA: do not draw them
##D                      nrpoints = 250, ret.selection=TRUE)
##D ## label the 20 very lowest-density points,the "outliers" (with obs.number):
##D i.20 <- i.s[1:20]
##D text(x[i.20,], labels = i.20, cex= 0.75)
##D 
##D ## somewhat similar, using identical smoothing computations,
##D ## but considerably *less* efficient for really large data:
##D plot(x, col = densCols(x), pch = 20)
##D 
##D ## use with pairs:
##D par(mfrow = c(1, 1))
##D y <- matrix(rnorm(40000), ncol = 4) + 3*rnorm(10000)
##D y[, c(2,4)] <-  -y[, c(2,4)]
##D pairs(y, panel = function(...) smoothScatter(..., nrpoints = 0, add = TRUE),
##D       gap = 0.2)
##D 
##D par(oldpar)
## End(No test)


