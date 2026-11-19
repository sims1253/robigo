x <- c(1.5, 2.5)
l <- vector("list", 200000)
for (i in 1:200000) l[[i]] <- x
cat(length(l), sum(lengths(l)), "\n")
