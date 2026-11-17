# Memory benchmark (G3 E2/E4, docs/ledger/g3.md): 1e6-element double vectors
# made and dropped 200 times (two 8 MB temporaries per iteration), so peak
# RSS shows how much garbage the collector lets pile up. scripts/rss.py.
# The row measures steady-state churn, not start-up or the JIT's phase: a
# full collection in the first iteration, after the loop is compiled, starts
# the collector's cycle at the same point in every build. Without it the peak
# follows how much start-up and compilation allocated (a JIT that folds 1:1e6
# into a 4 MB vector, or into a compact sequence, starts the cycle elsewhere),
# by up to 5.7% in robigo-rt and 35% in GNU R (benchmarks/rt/m1-bars.toml,
# churn's note).
s <- 0
for (k in 1:200) {
  if (k == 1L) invisible(gc())
  v <- (1:1e6) * 0.5 + k
  s <- s + v[k]
}
cat(s, "\n")
