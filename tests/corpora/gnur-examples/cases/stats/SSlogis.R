RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: SSlogis
### Title: Self-Starting 'nls' Logistic Model
### Aliases: SSlogis
### Keywords: models

### ** Examples
## No test: 
##D Chick.1 <- ChickWeight[ChickWeight$Chick == 1, ]
##D SSlogis(Chick.1$Time, 368, 14, 6)  # response only
##D local({
##D   Asym <- 368; xmid <- 14; scal <- 6
##D   SSlogis(Chick.1$Time, Asym, xmid, scal) # response _and_ gradient
##D })
##D getInitial(weight ~ SSlogis(Time, Asym, xmid, scal), data = Chick.1)
##D ## Initial values are in fact the converged one here, "Number of iter...: 0" :
##D fm1 <- nls(weight ~ SSlogis(Time, Asym, xmid, scal), data = Chick.1)
##D summary(fm1)
##D ## but are slightly improved here:
##D fm2 <- update(fm1, control=nls.control(tol = 1e-9, warnOnly=TRUE), trace = TRUE)
##D all.equal(coef(fm1), coef(fm2)) # "Mean relative difference: 9.6e-6"
##D str(fm2$convInfo) # 3 iterations
## End(No test)

dwlg1 <- data.frame(Prop = c(rep(0,5), 2, 5, rep(9, 9)), end = 1:16)
iPar <- getInitial(Prop ~ SSlogis(end, Asym, xmid, scal), data = dwlg1)
## failed in R <= 3.4.2 (because of the '0's in 'Prop')
stopifnot(all.equal(tolerance = 1e-6,
   iPar, c(Asym = 9.0678, xmid = 6.79331, scal = 0.499934)))

## Visualize the SSlogis()  model  parametrization :
  xx <- seq(-0.75, 5, by=1/32)
  yy <- 5 / (1 + exp((2-xx)/0.6)) # == SSlogis(xx, *):
  stopifnot( all.equal(yy, SSlogis(xx, Asym = 5, xmid = 2, scal = 0.6)) )
  require(graphics)
  op <- par(mar = c(0.5, 0, 3.5, 0))
  plot(xx, yy, type = "l", axes = FALSE, ylim = c(0,6), xlim = c(-1, 5),
       xlab = "", ylab = "", lwd = 2,
       main = "Parameters in the SSlogis model")
  mtext(quote(list(phi[1] == "Asym", phi[2] == "xmid", phi[3] == "scal")))
  usr <- par("usr")
  arrows(usr[1], 0, usr[2], 0, length = 0.1, angle = 25)
  arrows(0, usr[3], 0, usr[4], length = 0.1, angle = 25)
  text(usr[2] - 0.2, 0.1, "x", adj = c(1, 0))
  text(     -0.1, usr[4], "y", adj = c(1, 1))
  abline(h = 5, lty = 3)
  arrows(-0.8, c(2.1, 2.9),
         -0.8, c(0,   5  ), length = 0.1, angle = 25)
  text  (-0.8, 2.5, quote(phi[1]))
  segments(c(2,2.6,2.6), c(0,  2.5,3.5),   # NB.  SSlogis(x = xmid = 2) = 2.5
           c(2,2.6,2  ), c(2.5,3.5,2.5), lty = 2, lwd = 0.75)
  text(2, -.1, quote(phi[2]))
  arrows(c(2.2, 2.4), 2.5,
         c(2.0, 2.6), 2.5, length = 0.08, angle = 25)
  text(      2.3,     2.5, quote(phi[3])); text(2.7, 3, "1")
  par(op)



