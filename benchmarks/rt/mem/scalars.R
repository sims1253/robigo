# Memory benchmark (G3 D8, docs/ledger/g3/2026-11-17-m2m3-bars.md): 1e6 heap
# doubles kept live in a list, so peak RSS is mostly scalar headers.
# scripts/rss.py.
l <- vector("list", 1e6)
for (i in 1:1e6) l[[i]] <- i + 0.5
cat(length(l), l[[1e6]], "\n")
