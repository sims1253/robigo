# Memory benchmark (G3 D13, docs/ledger/g3/2026-11-17-m2m3-bars.md): 1e5
# three-element pairlists and 1e5 two-attribute integers kept live, so peak
# RSS is mostly LISTSXP nodes. scripts/rss.py.
p <- vector("list", 1e5)
x <- vector("list", 1e5)
for (i in 1:1e5) {
  p[[i]] <- as.pairlist(list(a = i, b = 2, c = 3))
  x[[i]] <- structure(i, a = 1, b = 2)
}
cat(length(p), length(x), "\n")
