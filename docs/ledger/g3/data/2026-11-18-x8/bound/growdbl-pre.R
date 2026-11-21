x <- numeric(1e6)
for (i in 1:1e6) x[i] <- i
cat(length(x), sum(x), "\n")
