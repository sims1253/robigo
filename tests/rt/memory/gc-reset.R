# L7 (amended 2026-11-24): gc(reset = TRUE) resets max used to used; after
# an allocation, max used is at least used and never decreases until reset.
# No absolute total is printed.

g0 <- gc(reset = TRUE)
cat("reset0_eq:", all(g0[, "max used"] == g0[, "used"]), "\n")

x <- numeric(2e5)
y <- .Internal(as.vector(1:5000, "list"))
g1 <- gc()
cat("alloc_ge_used:", all(g1[, "max used"] >= g1[, "used"]), "\n")
cat("alloc_gt_prev:", all(g1[, "max used"] > g0[, "max used"]), "\n")

rm(x, y)
g2 <- gc()
cat("drop_ge_used:", all(g2[, "max used"] >= g2[, "used"]), "\n")
cat("drop_no_decrease:", all(g2[, "max used"] >= g1[, "max used"]), "\n")
cat("drop_used_fell:", all(g2[, "used"] < g1[, "used"]), "\n")
cat("drop_max_gt_used:", all(g2[, "max used"] > g2[, "used"]), "\n")

g3 <- gc(reset = TRUE)
cat("reset3_eq:", all(g3[, "max used"] == g3[, "used"]), "\n")
cat("reset3_fell:", all(g3[, "max used"] < g2[, "max used"]), "\n")
