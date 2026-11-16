# Memory benchmark (G3 D10, docs/ledger/g3/2026-11-17-m2m3-bars.md): 1e6
# unique strings kept live. scripts/rss.py.
s <- paste0("s", 1:1e6)
cat(length(s), s[1e6], "\n")
