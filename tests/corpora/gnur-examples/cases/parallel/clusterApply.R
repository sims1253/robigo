RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(parallel)
### Name: clusterApply
### Title: Apply Operations using Clusters
### Aliases: clusterApply clusterApplyLB clusterCall clusterEvalQ
###   clusterExport clusterMap clusterSplit parApply parCapply parLapply
###   parRapply parSapply parLapplyLB parSapplyLB

### ** Examples
## No test: 
##D ## Use option cl.cores to choose an appropriate cluster size.
##D cl <- makeCluster(getOption("cl.cores", 2))
##D 
##D clusterApply(cl, 1:2, get("+"), 3)
##D xx <- 1
##D clusterExport(cl, "xx")
##D clusterCall(cl, function(y) xx + y, 2)
##D 
##D ## Use clusterMap like an mapply example
##D clusterMap(cl, function(x, y) seq_len(x) + y,
##D           c(a =  1, b = 2, c = 3), c(A = 10, B = 0, C = -10))
##D 
##D 
##D parSapply(cl, 1:20, get("+"), 3)
##D 
##D ## A bootstrapping example, which can be done in many ways:
##D clusterEvalQ(cl, {
##D   ## set up each worker.  Could also use clusterExport()
##D   library(boot)
##D   cd4.rg <- function(data, mle) MASS::mvrnorm(nrow(data), mle$m, mle$v)
##D   cd4.mle <- list(m = colMeans(cd4), v = var(cd4))
##D   NULL
##D })
##D res <- clusterEvalQ(cl, boot(cd4, corr, R = 100,
##D                     sim = "parametric", ran.gen = cd4.rg, mle = cd4.mle))
##D library(boot)
##D cd4.boot <- do.call(c, res)
##D boot.ci(cd4.boot,  type = c("norm", "basic", "perc"),
##D         conf = 0.9, h = atanh, hinv = tanh)
##D stopCluster(cl)
##D 
##D ## or
##D library(boot)
##D run1 <- function(...) {
##D    library(boot)
##D    cd4.rg <- function(data, mle) MASS::mvrnorm(nrow(data), mle$m, mle$v)
##D    cd4.mle <- list(m = colMeans(cd4), v = var(cd4))
##D    boot(cd4, corr, R = 500, sim = "parametric",
##D         ran.gen = cd4.rg, mle = cd4.mle)
##D }
##D cl <- makeCluster(mc <- getOption("cl.cores", 2))
##D ## to make this reproducible
##D clusterSetRNGStream(cl, 123)
##D cd4.boot <- do.call(c, parLapply(cl, seq_len(mc), run1))
##D boot.ci(cd4.boot,  type = c("norm", "basic", "perc"),
##D         conf = 0.9, h = atanh, hinv = tanh)
##D stopCluster(cl)
## End(No test)


