### Name: ts
### Title: Time-Series Objects
### Aliases: ts as.ts as.ts.default is.ts Ops.ts cbind.ts is.mts [.ts t.ts
###   head.ts tail.ts
### Keywords: ts

### ** Examples

require(graphics)

ts(1:10, frequency = 4, start = c(1959, 2)) # 2nd Quarter of 1959
print( ts(1:10, frequency = 7, start = c(12, 2)), calendar = TRUE)
# print.ts(.)
## Using July 1954 as start date:
gnp <- ts(cumsum(1 + round(rnorm(100), 2)),
          start = c(1954, 7), frequency = 12)
plot(gnp) # using 'plot.ts' for time-series plot

## No test: 
##D utils:: methods(class = "ts") # all functions with methods available for "ts"
## End(No test)

## "ts" methods for head() and tail()
utils:: head(lynx, 4)
utils:: tail(lynx, -7)
## Don't show: 
 stopifnot(is.ts(head(lynx, -6)),
                     is.ts(tail(lynx, 3)))
## End(Don't show)

## Multivariate
z <- ts(matrix(rnorm(300), 100, 3), start = c(1961, 1), frequency = 12)
class(z)
is.mts(z)
head(z) # "ts" method ==> incl. times:
plot(z)
plot(z, plot.type = "single", lty = 1:3)

## A phase plot:
plot(nhtemp, lag(nhtemp, 1), cex = .8, col = "blue",
     main = "Lag plot of New Haven temperatures")



