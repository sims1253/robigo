## GNU R 4.5.0 demo(smooth) -- src/library/stats/demo/smooth.R.
## Part of R, GPL-2 or later. Run as demo("smooth", package="stats",
## ask = FALSE, echo = TRUE) runs it: the demo() banner, then the file
## itself with echo (the corpus's echo mode supplies the prompts).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)
cat("\n\n\tdemo(smooth)\n\t---- ~~~~~~\n", sep = "")
### This used to be in   example(smooth) before we had package-specific demos
#  Copyright (C) 1997-2009 The R Core Team

require(stats); require(graphics); require(datasets)
op <- par(mfrow = c(1,1))

## The help(smooth) examples:
example(smooth, package="stats")

## Didactical investigation:

showSmooth <- function(x, leg.x = 1, leg.y = max(x)) {
  ss <- cbind(x, "3c"  = smooth(x, "3", end="copy"),
                 "3"   = smooth(x, "3"),
                 "3Rc" = smooth(x, "3R", end="copy"),
                 "3R"  = smooth(x, "3R"),
              sm = smooth(x))
  k <- ncol(ss) - 1
  n <- length(x)
  slwd <- c(1,1,4,1,3,2)
  slty <- c(0, 2:(k+1))
  matplot(ss, main = "Tukey Smoothers", ylab = "y ;  sm(y)",
          type= c("p",rep("l",k)), pch= par("pch"), lwd= slwd, lty= slty)
  legend(leg.x, leg.y,
         c("Data",       "3   (copy)", "3  (Tukey)",
                 "3R  (copy)", "3R (Tukey)", "smooth()"),
         pch= c(par("pch"),rep(-1,k)), col=1:(k+1), lwd= slwd, lty= slty)
  ss
}

## 4 simple didactical examples, showing different steps in smooth():

for(x in list(c(4, 6, 2, 2, 6, 3, 6, 6, 5, 2),
              c(3, 2, 1, 4, 5, 1, 3, 2, 4, 5, 2),
              c(2, 4, 2, 6, 1, 1, 2, 6, 3, 1, 6),
              x1))
    print(t(showSmooth(x)))

par(op)
