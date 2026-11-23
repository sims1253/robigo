# GNU R's compact sequences are lazy ALTREP objects: these need no memory.
x <- 1:1e15
cat(length(x), "\n")
cat(length(seq_len(1e15)), "\n")
