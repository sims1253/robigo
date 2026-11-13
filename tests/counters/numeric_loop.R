# A scalar arithmetic loop in a function: no allocation in the VM, and
# natively in the JIT, no VM op either.
#| vector_copies <= 0
#| envs <= 0
#| promises <= 0
#| vm,jit allocs <= 0
#| ast allocs <= 18
#| vm vm_ops <= 14
#| jit vm_ops <= 0
#| jit jit_deopts <= 0
f <- function(n) {
    s <- 0
    i <- 0L
    while (i < n) {
        i <- i + 1L
        s <- s + i * 0.5 - 1
    }
    s
}
invisible(f(n))
