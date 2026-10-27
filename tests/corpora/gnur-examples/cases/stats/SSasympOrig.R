RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: SSasympOrig
### Title: Self-Starting 'nls' Asymptotic Model through the Origin
### Aliases: SSasympOrig
### Keywords: models

### ** Examples
## No test: 
##D Lob.329 <- Loblolly[ Loblolly$Seed == "329", ]
##D SSasympOrig(Lob.329$age, 100, -3.2)  # response only
##D local({   Asym <- 100; lrc <- -3.2
##D   SSasympOrig(Lob.329$age, Asym, lrc) # response and gradient
##D })
##D getInitial(height ~ SSasympOrig(age, Asym, lrc), data = Lob.329)
##D ## Initial values are in fact the converged values
##D fm1 <- nls(height ~ SSasympOrig(age, Asym, lrc), data = Lob.329)
##D summary(fm1)
## End(No test)

## Visualize the SSasympOrig()  model  parametrization :

  xx <- seq(0, 5, length.out = 101)
  yy <- 5 * (1- exp(-xx * log(2)))
  stopifnot( all.equal(yy, SSasympOrig(xx, Asym = 5, lrc = log(log(2)))) )

  require(graphics)
  op <- par(mar = c(0, 0, 3.5, 0))
  plot(xx, yy, type = "l", axes = FALSE, ylim = c(0,5), xlim = c(-1/4, 5),
       xlab = "", ylab = "", lwd = 2,
       main = quote("Parameters in the SSasympOrig model"~~ f[phi](x)))
  mtext(quote(list(phi[1] == "Asym", phi[2] == "lrc")))
  usr <- par("usr")
  arrows(usr[1], 0, usr[2], 0, length = 0.1, angle = 25)
  arrows(0, usr[3], 0, usr[4], length = 0.1, angle = 25)
  text(usr[2] - 0.2, 0.1, "x", adj = c(1, 0))
  text(   -0.1,   usr[4], "y", adj = c(1, 1))
  abline(h = 5, lty = 3)
  axis(2, at = 5*c(1/2,1), labels= expression(frac(phi[1],2), phi[1]), pos=0, las=1)
  arrows(c(.3,.7), 5/2,
         c(0, 1 ), 5/2, length = 0.08, angle = 25)
  text(   0.5,     5/2, quote(t[0.5]))
  text(   1 +.4,   5/2,
       quote({f(t[0.5]) == frac(phi[1],2)}~{} %=>% {}~~{t[0.5] == frac(log(2), e^{phi[2]})}),
       adj = c(0, 0.5))
  par(op)



