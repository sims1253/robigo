# .Random.seed and set.seed with compact sequences (AC1): GetRNGkind's
# INTEGER(seeds) reads them expanded
show <- function(expr) withCallingHandlers(tryCatch(expr,
    error = function(e) cat("Error:", conditionMessage(e), "\n")),
    warning = function(w) { cat("Warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
set.seed(1:3)
.Random.seed[1:3]
set.seed(seq_len(2))
.Random.seed[1:3]
.Random.seed <- 10403:11028
RNGkind()
.Random.seed[1:4]
.Random.seed <- c(10403L, seq_len(625))
set.seed(4)
.Random.seed[1:3]
.Random.seed <- 10400:10402
show(RNGkind())
.Random.seed <- 10400:10403
RNGkind()
.Random.seed
.Random.seed <- seq_len(1)
show(RNGkind())
length(.Random.seed)
.Random.seed <- 20000:20003
show(RNGkind())
.Random.seed[1]
show(set.seed(1, kind = 1:3))
RNGkind()
