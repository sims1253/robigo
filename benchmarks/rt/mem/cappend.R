# Memory and wall benchmark (G3 D12's control, docs/ledger/g3/2026-11-17-m2m3-bars.md):
# a double vector grown by `x <- c(x, i)` to 2e4 elements, a new vector each
# time, which page remapping cannot help. scripts/rss.py.
x <- c()
for (i in 1:2e4) x <- c(x, i)
cat(length(x), sum(x), "\n")
