n <- 500000
x <- numeric(n)
for (i in seq_len(n)) x[i] <- i * 0.5
print(sum(x))
