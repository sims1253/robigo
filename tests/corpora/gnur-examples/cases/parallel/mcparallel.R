RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(parallel)
### Name: mcparallel
### Title: Evaluate an R Expression Asynchronously in a Separate Process
### Aliases: mccollect mcparallel
### Keywords: interface

### ** Examples

p <- mcparallel(1:10)
q <- mcparallel(1:20)
# wait for both jobs to finish and collect all results
res <- mccollect(list(p, q))

## IGNORE_RDIFF_BEGIN
p <- mcparallel(1:10)
mccollect(p, wait = FALSE, 10) # will retrieve the result (since it's fast)
mccollect(p, wait = FALSE)     # will signal the job as terminating
mccollect(p, wait = FALSE)     # there is no longer such a job
## IGNORE_RDIFF_END
## Don't show: 
set.seed(123, "L'Ecuyer"); mc.reset.stream()
## End(Don't show)
# a naive parallel lapply can be created using mcparallel alone:
jobs <- lapply(1:10, function(x) mcparallel(rnorm(x), name = x))
mccollect(jobs)



