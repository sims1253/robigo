# L7 (amended 2026-11-24): gc() and gc(full = FALSE) table shape, types,
# dimnames, limit column, and (Mb) unit relations. No absolute total is printed.

invisible(mem.maxVSize(Inf))
invisible(mem.maxNSize(Inf))

mb <- function(cells, sz) 0.1 * ceiling(10 * cells / 1048576 * sz)

check_table <- function(g, label) {
  cat(label, "class:", class(g), "typeof:", typeof(g), "dim:", dim(g), "\n")
  print(dimnames(g))
  cat(label, "finite_nonneg:", all(is.finite(g) & g >= 0), "\n")
  cat(label, "ncells_mb:", all(abs(g["Ncells", c(2, 4, 6)] - mb(g["Ncells", c(1, 3, 5)], 56)) < 1e-12), "\n")
  cat(label, "vcells_mb:", all(abs(g["Vcells", c(2, 4, 6)] - mb(g["Vcells", c(1, 3, 5)], 8)) < 1e-12), "\n")
}

check_table(gc(), "full")
check_table(gc(full = FALSE), "lite")

# With a vector limit set, the "limit (Mb)" column appears as column 5.
g0 <- gc()
lim_v <- ceiling(g0["Vcells", "gc trigger"] * 8 / 1048576) + 64
invisible(mem.maxVSize(lim_v))
gv <- gc()
cat("vlimit dim:", dim(gv), "typeof:", typeof(gv), "\n")
print(dimnames(gv))
cat("vlimit na_n:", is.na(gv["Ncells", "limit (Mb)"]),
    "val_v:", abs(gv["Vcells", "limit (Mb)"] - 0.1 * ceiling(10 * lim_v)) < 1e-12, "\n")

# With both vector and node limits set, both rows of "limit (Mb)" are populated.
lim_n <- g0["Ncells", "gc trigger"] + 1e6
invisible(mem.maxNSize(lim_n))
gnv <- gc()
cat("both dim:", dim(gnv),
    "val_n:", abs(gnv["Ncells", "limit (Mb)"] - mb(lim_n, 56)) < 1e-12,
    "val_v:", abs(gnv["Vcells", "limit (Mb)"] - 0.1 * ceiling(10 * lim_v)) < 1e-12, "\n")

# Lifting only the vector limit keeps the limit column for Ncells.
invisible(mem.maxVSize(Inf))
gn <- gc()
cat("nlimit dim:", dim(gn),
    "na_v:", is.na(gn["Vcells", "limit (Mb)"]),
    "val_n:", abs(gn["Ncells", "limit (Mb)"] - mb(lim_n, 56)) < 1e-12, "\n")

# Lifting both restores the 6-column table.
invisible(mem.maxNSize(Inf))
cat("unlifted dim:", dim(gc()), "\n")
