# L7 (amended 2026-11-24): ported AdjustHeapSize trigger rules as relations:
# trigger >= used after collection, monotone growth on a growing vector heap,
# and shrinkage once the heap drops back down. No trigger value is printed.

invisible(mem.maxVSize(Inf))

g0 <- gc()
cat("g0_ge_used:", all(g0[, "gc trigger"] >= g0[, "used"]), "\n")

v0 <- g0["Vcells", "gc trigger"]
big1 <- numeric(as.integer(ceiling(v0 * 0.8)))
g1 <- gc()
cat("g1_ge_used:", all(g1[, "gc trigger"] >= g1[, "used"]), "\n")
cat("g1_grew:", g1["Vcells", "gc trigger"] > v0, "\n")

big2 <- numeric(as.integer(ceiling(g1["Vcells", "gc trigger"] * 0.8)))
g2 <- gc()
cat("g2_ge_used:", all(g2[, "gc trigger"] >= g2[, "used"]), "\n")
cat("g2_grew:", g2["Vcells", "gc trigger"] > g1["Vcells", "gc trigger"], "\n")

rm(big1, big2)
g3 <- gc()
cat("g3_ge_used:", all(g3[, "gc trigger"] >= g3[, "used"]), "\n")
cat("g3_shrank:", g3["Vcells", "gc trigger"] < g2["Vcells", "gc trigger"], "\n")

prev <- g3["Vcells", "gc trigger"]
monotone <- TRUE
for (i in 1:6) {
  gi <- gc()
  cur <- gi["Vcells", "gc trigger"]
  if (cur > prev || cur < gi["Vcells", "used"]) monotone <- FALSE
  prev <- cur
}
cat("shrink_monotone:", monotone, "\n")
cat("shrink_below_peak:", prev < g3["Vcells", "gc trigger"], "\n")
