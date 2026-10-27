RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: densCols
### Title: Colors for Smooth Density Plots
### Aliases: densCols blues9
### Keywords: dplot

### ** Examples
## No test: 
##D x1  <- matrix(rnorm(1e3), ncol = 2)
##D x2  <- matrix(rnorm(1e3, mean = 3, sd = 1.5), ncol = 2)
##D x   <- rbind(x1, x2)
##D 
##D dcols <- densCols(x)
##D graphics::plot(x, col = dcols, pch = 20, main = "n = 1000")
## End(No test)


