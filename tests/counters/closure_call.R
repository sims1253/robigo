# A loop calling a small closure without free variables: the VM and the JIT
# inline it (no frame, no promise); the interpreter makes one frame and a
# promise per argument.
#| vector_copies <= 0
#| vm,jit envs <= 0
#| vm,jit promises <= 0
#| vm,jit allocs <= 0
#| ast envs <= 1
#| ast promises <= 2
#| ast allocs <= 7
#| vm vm_ops <= 4
#| jit vm_ops <= 0
#| jit jit_deopts <= 0
add <- function(a, b) a + b
s <- 0
for (i in seq_len(n)) s <- add(s, i)
