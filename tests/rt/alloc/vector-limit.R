# E9 (docs/g3.md §15.3): the vector memory limit (memory.c's mem_err_heap,
# set here through mem.maxVSize's .Internal) and a request no system serves
# (allocVector3's "cannot allocate vector of size"), in GNU R's words.
m <- function(e) tryCatch(e, error = function(c) cat("Error:", conditionMessage(c), "\n"))
invisible(.Internal(mem.maxVSize(200)))
.Internal(mem.maxVSize(0))
m(x <- rep(0, 5e7))
y <- rep(0, 1e6)
length(y)
invisible(.Internal(mem.maxVSize(Inf)))
.Internal(mem.maxVSize(0))
m(z <- rep(0, 1e15))
invisible(.Internal(mem.maxVSize(1)))
cat("done\n")
