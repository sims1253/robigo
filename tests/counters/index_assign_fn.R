# x[i] <- v on a function's local vector: as at top level (index_assign.R).
#| vector_copies <= 0
#| envs <= 0
#| promises <= 0
#| vm,jit allocs <= 0
#| ast allocs <= 21
#| vm vm_ops <= 11
#| jit vm_ops <= 0
#| jit jit_deopts <= 0
f <- function(n) {
    x <- numeric(1000)
    for (i in seq_len(n)) x[(i - 1L) %% 1000L + 1L] <- i
    x
}
invisible(f(n))
