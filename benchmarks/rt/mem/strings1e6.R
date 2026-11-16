# Memory benchmark (D10, docs/ledger/g3/2026-11-17-d10-bar.md): 1e6 unique
# strings made by paste0, with no use that needs their pointers. Peak RSS by
# scripts/rss.py.
x <- paste0("u", 1:1000000)
cat(length(x), x[1000000], "\n")
