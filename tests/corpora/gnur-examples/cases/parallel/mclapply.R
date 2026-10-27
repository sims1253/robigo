library(parallel)
### Name: mclapply
### Title: Parallel Versions of 'lapply' and 'mapply' using Forking
### Aliases: mclapply mcmapply mcMap
### Keywords: interface

### ** Examples
## No test: 
##D simplify2array(mclapply(rep(4, 5), rnorm))
##D # use the same random numbers for all values
##D set.seed(1)
##D simplify2array(mclapply(rep(4, 5), rnorm, mc.preschedule = FALSE,
##D                         mc.set.seed = FALSE))
##D 
##D ## Contrast this with the examples for clusterCall
##D library(boot)
##D cd4.rg <- function(data, mle) MASS::mvrnorm(nrow(data), mle$m, mle$v)
##D cd4.mle <- list(m = colMeans(cd4), v = var(cd4))
##D mc <- getOption("mc.cores", 2)
##D run1 <- function(...) boot(cd4, corr, R = 500, sim = "parametric",
##D                            ran.gen = cd4.rg, mle = cd4.mle)
##D ## To make this reproducible:
##D set.seed(123, "L'Ecuyer")
##D res <- mclapply(seq_len(mc), run1)
##D cd4.boot <- do.call(c, res)
##D boot.ci(cd4.boot,  type = c("norm", "basic", "perc"),
##D         conf = 0.9, h = atanh, hinv = tanh)
##D 
##D ## Usage of the affinity.list parameter
##D A <- runif(2500000,0,100)
##D B <- runif(2500000,0,100)
##D C <- runif(5000000,0,100)
##D first <- function(i) head(sort(i), n = 1)
##D 
##D # Restrict all elements of X to run on CPU 1 and 2
##D affL <- list(c(1,2), c(1,2), c(1,2))
##D mclapply(list(A, A, A), first, mc.preschedule = FALSE, affinity.list = affL)
##D 
##D 
##D # Completion times are assumed to have a high variance
##D # To optimize the overall execution time elements of X are scheduled to suitable CPUs
##D # Assuming that the runtime for C is as long as the runtime of A plus B
##D # mapping: A to 1 , B to 1, C to 2
##D X <- list(A, B, C)
##D affL <- c(1, 1, 2)
##D mclapply(X, first, mc.preschedule = FALSE, affinity.list = affL)
##D 
## End(No test)


