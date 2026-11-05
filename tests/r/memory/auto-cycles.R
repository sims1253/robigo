# Environments that point at themselves are garbage once the loop drops
# them, and the program never calls gc(): the collections the growing heap
# triggers reclaim them and run their finalizers (GNU R: memory.c's
# R_gc_internal when the vector heap is full, then R_RunPendingFinalizers).
n <- 0L
for (i in 1:20000) {
    e <- new.env()
    e$self <- e
    e$f <- function() e
    e$x <- numeric(1000)
    reg.finalizer(e, function(e) n <<- n + 1L)
}
x <- numeric(1e6)
for (i in 1:10) x[i] <- i
cat(n > 0L, "\n")
