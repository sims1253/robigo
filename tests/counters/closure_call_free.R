# A loop calling a closure with a free variable: the VM makes its frame and
# one promise per call, the JIT runs it natively with neither.
#| vector_copies <= 0
#| ast,vm envs <= 1
#| ast,vm promises <= 1
#| jit envs <= 0
#| jit promises <= 0
#| ast allocs <= 9
#| vm allocs <= 2
#| jit allocs <= 0
#| vm vm_ops <= 9
#| jit vm_ops <= 0
#| jit jit_deopts <= 0
k <- 2
scale <- function(a) a * k
s <- 0
for (i in seq_len(n)) s <- s + scale(i)
