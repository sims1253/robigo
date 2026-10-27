RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: JohnsonJohnson
### Title: Quarterly Earnings per Johnson & Johnson Share
### Aliases: JohnsonJohnson
### Keywords: datasets

### ** Examples
## No test: 
##D require(stats); require(graphics)
##D JJ <- log10(JohnsonJohnson)
##D plot(JJ)
##D ## This example gives a possible-non-convergence warning on some
##D ## platforms, but does seem to converge on x86 Linux and Windows.
##D (fit <- StructTS(JJ, type = "BSM"))
##D tsdiag(fit)
##D sm <- tsSmooth(fit)
##D plot(cbind(JJ, sm[, 1], sm[, 3]-0.5), plot.type = "single",
##D      col = c("black", "green", "blue"))
##D abline(h = -0.5, col = "grey60")
##D 
##D monthplot(fit)
## End(No test)


