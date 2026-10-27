RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: chull
### Title: Compute Convex Hull of a Set of Points
### Aliases: chull
### Keywords: graphs

### ** Examples

X <- matrix(stats::rnorm(2000), ncol = 2)
chull(X)

plot(X, cex = 0.5)
polygon(X[chull(X), ])



