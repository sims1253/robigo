RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: findInterval
### Title: Find Interval Numbers or Indices
### Aliases: findInterval
### Keywords: arith utilities

### ** Examples

x <- 2:18
v <- c(5, 10, 15) # create two bins [5,10) and [10,15)
cbind(x, findInterval(x, v))

N <- 100
X <- sort(round(stats::rt(N, df = 2), 2))
tt <- c(-100, seq(-2, 2, length.out = 201), +100)
it <- findInterval(tt, X)
tt[it < 1 | it >= N] # only first and last are outside range(X)
stopifnot(identical(it, ## suppressing the checks is faster *BUT* dangerous, unless
                    ##     you *know* that X is sorted   and   tt contains no NA's
                    findInterval(tt, X, checkSorted=FALSE, checkNA=FALSE)))

##  'left.open = TRUE' means  "mirroring" :
N <- length(v)
stopifnot(identical(
                  findInterval( x,  v,  left.open=TRUE) ,
              N - findInterval(-x, -v[N:1])))



