RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(parallel)
### Name: RNGstreams
### Title: Implementation of Pierre L'Ecuyer's RngStreams
### Aliases: nextRNGStream nextRNGSubStream clusterSetRNGStream
###   mc.reset.stream
### Keywords: distribution sysdata

### ** Examples

RNGkind("L'Ecuyer-CMRG")
set.seed(123)
(s <- .Random.seed)
## do some work involving random numbers.
nextRNGStream(s)
nextRNGSubStream(s)



