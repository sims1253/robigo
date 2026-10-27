### Name: smooth.spline
### Title: Fit a Smoothing Spline
### Aliases: smooth.spline .nknots.smspl
### Keywords: smooth

### ** Examples

require(graphics)
plot(dist ~ speed, data = cars, main = "data(cars)  &  smoothing splines")
cars.spl <- with(cars, smooth.spline(speed, dist))
cars.spl
## This example has duplicate points, so avoid cv = TRUE
## Don't show: 
  stopifnot(cars.spl $ w == table(cars$speed)) # weights = multiplicities
  utils::str(cars.spl, digits.d = 5, vec.len = 6)
  cars.spl$fit
## End(Don't show)
lines(cars.spl, col = "blue")
ss10 <- smooth.spline(cars[,"speed"], cars[,"dist"], df = 10)
lines(ss10, lty = 2, col = "red")
legend(5,120,c(paste("default [C.V.] => df =",round(cars.spl$df,1)),
               "s( * , df = 10)"), col = c("blue","red"), lty = 1:2,
       bg = 'bisque')


## Residual (Tukey Anscombe) plot:
plot(residuals(cars.spl) ~ fitted(cars.spl))
abline(h = 0, col = "gray")

## consistency check:
stopifnot(all.equal(cars$dist,
                    fitted(cars.spl) + residuals(cars.spl)))
## The chosen inner knots in original x-scale :
with(cars.spl$fit, min + range * knot[-c(1:3, nk+1 +1:3)]) # == unique(cars$speed)

## Visualize the behavior of  .nknots.smspl()
nKnots <- Vectorize(.nknots.smspl) ; c.. <- adjustcolor("gray20",.5)
curve(nKnots, 1, 250, n=250)
abline(0,1, lty=2, col=c..); text(90,90,"y = x", col=c.., adj=-.25)
abline(h=100,lty=2); abline(v=200, lty=2)

n <- c(1:799, seq(800, 3490, by=10), seq(3500, 10000, by = 50))
plot(n, nKnots(n), type="l", main = "Vectorize(.nknots.smspl) (n)")
abline(0,1, lty=2, col=c..); text(180,180,"y = x", col=c..)
n0 <- c(50, 200, 800, 3200); c0 <- adjustcolor("blue3", .5)
lines(n0, nKnots(n0), type="h", col=c0)
axis(1, at=n0, line=-2, col.ticks=c0, col=NA, col.axis=c0)
axis(4, at=.nknots.smspl(10000), line=-.5, col=c..,col.axis=c.., las=1)

##-- artificial example
y18 <- c(1:3, 5, 4, 7:3, 2*(2:5), rep(10, 4))
xx  <- seq(1, length(y18), length.out = 201)
(s2   <- smooth.spline(y18)) # GCV
(s02  <- smooth.spline(y18, spar = 0.2))
(s02. <- smooth.spline(y18, spar = 0.2, cv = NA))
plot(y18, main = deparse(s2$call), col.main = 2)
lines(s2, col = "gray"); lines(predict(s2, xx), col = 2)
lines(predict(s02, xx), col = 3); mtext(deparse(s02$call), col = 3)

## Specifying 'lambda' instead of usual spar :
(s2. <- smooth.spline(y18, lambda = s2$lambda, tol = s2$tol))

## Don't show: 
stopifnot(identical(
    with(s2$fit, min + range * knot[-c(1:3, nk+1+1:3)]),
    as.numeric(1:18)),
    with(cars.spl$fit, min + range * knot[-c(1:3, nk+1+1:3)]) == unique(cars$speed))

nD <- c("spar", "ratio", "iparms", "call"); nn <- setdiff(names(s2), nD)
stopifnot(all.equal(s2[nn], s2.[nn], tolerance = 7e-7), # seen 6.86e-8
          all.equal(predict(s02 , xx),
		    predict(s02., xx), tolerance = 1e-15))
## End(Don't show)
## No test: 
##D ## The following shows the problematic behavior of 'spar' searching:
##D (s2  <- smooth.spline(y18, control =
##D                       list(trace = TRUE, tol = 1e-6, low = -1.5)))
##D (s2m <- smooth.spline(y18, cv = TRUE, control =
##D                       list(trace = TRUE, tol = 1e-6, low = -1.5)))
##D ## both above do quite similarly (Df = 8.5 +- 0.2)
## End(No test)


