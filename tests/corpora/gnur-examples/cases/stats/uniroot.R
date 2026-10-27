RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: uniroot
### Title: One Dimensional Root (Zero) Finding
### Aliases: uniroot
### Keywords: optimize

### ** Examples
## No test: 
##D require(utils) # for str
##D 
##D ## some platforms hit zero exactly on the first step:
##D ## if so the estimated precision is 2/3.
##D f <- function (x, a) x - a
##D str(xmin <- uniroot(f, c(0, 1), tol = 0.0001, a = 1/3))
##D 
##D ## handheld calculator example: fixed point of cos(.):
##D uniroot(function(x) cos(x) - x, lower = -pi, upper = pi, tol = 1e-9)$root
##D 
##D str(uniroot(function(x) x*(x^2-1) + .5, lower = -2, upper = 2,
##D             tol = 0.0001))
##D str(uniroot(function(x) x*(x^2-1) + .5, lower = -2, upper = 2,
##D             tol = 1e-10))
##D 
##D ## Find the smallest value x for which exp(x) > 0 (numerically):
##D r <- uniroot(function(x) 1e80*exp(x) - 1e-300, c(-1000, 0), tol = 1e-15)
##D str(r, digits.d = 15) # around -745, depending on the platform.
##D 
##D exp(r$root)     # = 0, but not for r$root * 0.999...
##D minexp <- r$root * (1 - 10*.Machine$double.eps)
##D exp(minexp)     # typically denormalized
## End(No test)

##--- uniroot() with new interval extension + checking features: --------------

f1 <- function(x) (121 - x^2)/(x^2+1)
f2 <- function(x) exp(-x)*(x - 12)

try(uniroot(f1, c(0,10)))
try(uniroot(f2, c(0, 2)))
##--> error: f() .. end points not of opposite sign

## where as  'extendInt="yes"'  simply first enlarges the search interval:
u1 <- uniroot(f1, c(0,10),extendInt="yes", trace=1)
u2 <- uniroot(f2, c(0,2), extendInt="yes", trace=2)
stopifnot(all.equal(u1$root, 11, tolerance = 1e-5),
          all.equal(u2$root, 12, tolerance = 6e-6))

## The *danger* of interval extension:
## No way to find a zero of a positive function, but
## numerically, f(-|M|) becomes zero :
u3 <- uniroot(exp, c(0,2), extendInt="yes", trace=TRUE)

## Nonsense example (must give an error):
tools::assertCondition( uniroot(function(x) 1, 0:1, extendInt="yes"),
                       "error", verbose=TRUE)

## Convergence checking :
sinc <- function(x) ifelse(x == 0, 1, sin(x)/x)
curve(sinc, -6,18); abline(h=0,v=0, lty=3, col=adjustcolor("gray", 0.8))
## Don't show: 
tools::assertWarning(
## End(Don't show)
uniroot(sinc, c(0,5), extendInt="yes", maxiter=4) #-> "just" a warning
## Don't show: 
 , verbose=TRUE)
## End(Don't show)

## now with  check.conv=TRUE, must signal a convergence error :
## Don't show: 
tools::assertError(
## End(Don't show)
uniroot(sinc, c(0,5), extendInt="yes", maxiter=4, check.conv=TRUE)
## Don't show: 
 , verbose=TRUE)
## End(Don't show)

### Weibull cumulative hazard (example origin, Ravi Varadhan):
cumhaz <- function(t, a, b) b * (t/b)^a
froot <- function(x, u, a, b) cumhaz(x, a, b) - u

n <- 1000
u <- -log(runif(n))
a <- 1/2
b <- 1
## Find failure times
ru <- sapply(u, function(x)
   uniroot(froot, u=x, a=a, b=b, interval= c(1.e-14, 1e04),
           extendInt="yes")$root)
ru2 <- sapply(u, function(x)
   uniroot(froot, u=x, a=a, b=b, interval= c(0.01,  10),
           extendInt="yes")$root)
stopifnot(all.equal(ru, ru2, tolerance = 6e-6))

r1 <- uniroot(froot, u= 0.99, a=a, b=b, interval= c(0.01, 10),
             extendInt="up")
stopifnot(all.equal(0.99, cumhaz(r1$root, a=a, b=b)))

## An error if 'extendInt' assumes "wrong zero-crossing direction":
## Don't show: 
tools::assertError(
## End(Don't show)
uniroot(froot, u= 0.99, a=a, b=b, interval= c(0.1, 10), extendInt="down")
## Don't show: 
 , verbose=TRUE)
## End(Don't show)



