# L7 (amended 2026-11-24): mem.maxVSize() and mem.maxNSize() read/set/Inf,
# warnings for invalid or too-small values, the limit column, and the
# vector and cons memory limit errors. No absolute total is printed.

invisible(mem.maxVSize(Inf))
invisible(mem.maxNSize(Inf))

cat("initial:", mem.maxVSize(), mem.maxNSize(), "\n")

# Invalid (non-numeric) and non-positive values leave the limit unchanged.
cat("v_bad:", mem.maxVSize("abc"), mem.maxVSize(-1), mem.maxVSize(0), mem.maxVSize(NA), "\n")
cat("n_bad:", mem.maxNSize("abc"), mem.maxNSize(-1), mem.maxNSize(0), mem.maxNSize(NA), "\n")

# Limits below current usage warn and are ignored.
cat("v_low:", mem.maxVSize(1), "\n")
cat("n_low:", mem.maxNSize(1), "\n")

# Fixed vector limit above startup trigger: exact error message and limit column.
cat("v_set:", mem.maxVSize(128), "\n")
cat("v_col:", gc()["Vcells", "limit (Mb)"], "\n")
v_err <- tryCatch({ x <- numeric(3e7); "ok" }, error = function(e) conditionMessage(e))
cat("v_err:", v_err, "\n")
cat("v_inf:", mem.maxVSize(Inf), "\n")

# Vector limit just above current trigger: message names the set limit.
g <- gc()
lim_v <- ceiling(g["Vcells", "gc trigger"] * 8 / 1048576) + 2
invisible(mem.maxVSize(lim_v))
v_near <- tryCatch({ x <- numeric(as.integer(lim_v * 1048576 / 8) + 1e5); "ok" },
                   error = function(e) conditionMessage(e))
expected_v <- sprintf("vector memory limit of %.1f Mb reached, see mem.maxVSize()", lim_v)
cat("v_near_match:", identical(v_near, expected_v), "\n")
invisible(mem.maxVSize(Inf))

# Node limit just above current trigger: read/set/Inf, limit column, and error.
g <- gc()
lim_n <- g["Ncells", "gc trigger"] + 2000
cat("n_set_match:", mem.maxNSize(lim_n) == lim_n, "\n")
cat("n_col_match:", abs(gc()["Ncells", "limit (Mb)"] - 0.1 * ceiling(10 * lim_n / 1048576 * 56)) < 1e-12, "\n")
# Allocating ~350k cons cells through as.pairlist() (which collects between
# elements, as GNU R's cons() does) would take ~350k collections under
# gctorture; turn torture off for the limit-crossing allocation itself
# (tests/rt/coerce/list-pairlist-collect.R covers as.pairlist under torture).
old_torture <- gctorture(FALSE)
n_over <- as.integer(lim_n - g["Ncells", "used"] + 50000)
n_err <- tryCatch({ x <- as.pairlist(seq_len(n_over)); "ok" },
                  error = function(e) conditionMessage(e),
                  finally = gctorture(old_torture))
expected_n <- sprintf("cons memory limit of %.0f nodes reached, see mem.maxNSize()", lim_n)
cat("n_err_match:", identical(n_err, expected_n), "\n")
cat("n_inf:", mem.maxNSize(Inf), "\n")
