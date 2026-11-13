# l$a[i] <- v on a list nothing else holds updates the element in place: no
# copy in any tier. Like x[i] <- v (index_assign.R) it should allocate
# nothing in the VM and run natively in the JIT.
#| vector_copies <= 0
#| envs <= 0
#| promises <= 0
#| vm,jit allocs <= 0
#| ast allocs <= 37
#| vm vm_ops <= 4
#| jit vm_ops <= 0
#| jit jit_deopts <= 0
l <- list(a = numeric(1000), b = "b")
for (i in seq_len(n)) l$a[(i - 1L) %% 1000L + 1L] <- i
