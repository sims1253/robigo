# Memory and wall benchmark (G3 D12, docs/ledger/g3/2026-11-17-m2m3-bars.md):
# a double vector grown by `x[i] <- i` to 1e6 elements (subassign.c
# EnlargeVector). scripts/rss.py.
x <- numeric(0)
for (i in 1:1e6) x[i] <- i
cat(length(x), sum(x), "\n")
