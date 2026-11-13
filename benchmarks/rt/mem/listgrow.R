# Memory benchmark (G3 E2/E4, docs/ledger/g3.md): a list grown one element
# at a time by `[[<-` past its end (subassign.c EnlargeVector), every element
# the same shared vector. Peak RSS by scripts/rss.py.
x <- c(1.5, 2.5)
l <- list()
for (i in 1:200000) l[[i]] <- x
cat(length(l), sum(lengths(l)), "\n")
