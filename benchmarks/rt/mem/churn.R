# Memory benchmark (G3 E2/E4, docs/ledger/g3.md): 1e6-element double vectors
# made and dropped 200 times (two 8 MB temporaries per iteration), so peak
# RSS shows how much garbage the collector lets pile up. scripts/rss.py.
s <- 0
for (k in 1:200) {
  v <- (1:1e6) * 0.5 + k
  s <- s + v[k]
}
cat(s, "\n")
