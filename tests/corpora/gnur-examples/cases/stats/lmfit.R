### Name: lm.fit
### Title: Fitter Functions for Linear Models
### Aliases: lm.fit lm.wfit .lm.fit
### Keywords: regression array

### ** Examples

require(utils)
set.seed(129)

n <- 7 ; p <- 2
X <- matrix(rnorm(n * p), n, p) # no intercept!
y <- rnorm(n)
w <- rnorm(n)^2

str(lmw <- lm.wfit(x = X, y = y, w = w))

str(lm. <- lm.fit (x = X, y = y))
## Don't show: 
  ## These are the same calculations at C level, but a parallel BLAS
  ## might not do them the same way twice, and if seems serial MKL does not.
  lm.. <- .lm.fit(X,y)
  lm.w <- .lm.fit(X*sqrt(w), y*sqrt(w))
  id <- function(x, y) all.equal(x, y, tolerance = 1e-15, scale = 1)
  stopifnot(id(unname(lm.$coefficients), lm..$coefficients),
	    id(unname(lmw$coefficients), lm.w$coefficients))
## End(Don't show)
## fits w/o intercept:
all.equal(unname(coef(lm(y ~ X-1))),
          unname(coef( lm.fit(X,y))))
all.equal(unname(coef( lm.fit(X,y))),
                 coef(.lm.fit(X,y)))
## No test: 
##D if(require("microbenchmark")) {
##D   mb <- microbenchmark(lm(y~X-1), lm.fit(X,y), .lm.fit(X,y))
##D   print(mb)
##D   boxplot(mb, notch=TRUE)
##D }
## End(No test)



