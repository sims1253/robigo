# GC at scale: robigo's heap vs GNU R's. Run by scale.sh in both.
dyn.load("scale.so")
peak <- function() {
  s <- readLines("/proc/self/status")
  as.numeric(sub("[^0-9]*([0-9]+).*", "\\1", s[startsWith(s, "VmHWM")])) / 1024
}
gct <- function() {
  r <- .Call("scale_gc_seconds")
  if (is.na(r)) sum(gc.time()[1:2]) else r
}
t0 <- proc.time()[[3]]; g0 <- gct()
live <- .Call("scale_live", 10000000L)
t1 <- proc.time()[[3]]; g1 <- gct()
rss <- .Call("scale_rss")
invisible(.Call("scale_churn", 5e7))
t2 <- proc.time()[[3]]; g2 <- gct()
cat(sprintf("build 10M live strings: %.2f s (gc %.2f s)\n", t1 - t0, g1 - g0))
cat(sprintf("churn 50M small vectors over it: %.2f s (gc %.2f s)\n", t2 - t1, g2 - g1))
cat(sprintf("RSS after building in C %.0f MB, after return to R %.0f MB\n", rss[1], rss[2]))
cat(sprintf("peak RSS %.0f MB; collections (robigo) %s\n", peak(), format(.Call("scale_gc_count"))))
invisible(length(live))
