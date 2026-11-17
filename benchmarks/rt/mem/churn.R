# Memory benchmark (G3 E2/E4, docs/ledger/g3.md): 1e6-element double vectors
# made and dropped 200 times (two 8 MB temporaries per iteration), so peak
# RSS shows how much garbage the collector lets pile up. scripts/rss.py.
# A full collection first, so that the loop starts at the same point of the
# collector's cycle in every build: without it the peak depends on how much
# the start-up allocated, and a few MiB of it moved the peak by up to 5.7% in
# robigo-rt and 35% in GNU R (benchmarks/rt/m1-bars.toml, churn's note).
invisible(gc())
s <- 0
for (k in 1:200) {
  v <- (1:1e6) * 0.5 + k
  s <- s + v[k]
}
cat(s, "\n")
