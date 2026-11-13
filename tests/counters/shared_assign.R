# y[i] <- v on a vector shared with x copies it once, and only once (the JIT
# keeps y in a buffer of its own and copies nothing).
#| ast,vm vector_copies <= 1
#| jit vector_copies <= 0
#| envs <= 0
#| promises <= 0
#| ast allocs <= 15
#| vm allocs <= 2
#| jit allocs <= 0
#| vm vm_ops <= 9
#| jit vm_ops <= 0
#| jit jit_deopts <= 0
x <- numeric(10)
for (i in seq_len(n)) {
    y <- x
    y[1L] <- i
}
