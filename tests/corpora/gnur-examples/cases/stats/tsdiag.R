RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: tsdiag
### Title: Diagnostic Plots for Time-Series Fits
### Aliases: tsdiag tsdiag.arima0 tsdiag.Arima tsdiag.StructTS
### Keywords: ts

### ** Examples

## No test: 
##D require(graphics)
##D 
##D fit <- arima(lh, c(1,0,0))
##D tsdiag(fit)
##D 
##D ## see also examples(arima)
##D 
##D (fit <- StructTS(log10(JohnsonJohnson), type = "BSM"))
##D tsdiag(fit)
## End(No test)


