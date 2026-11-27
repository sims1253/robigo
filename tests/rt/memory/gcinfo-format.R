# L7 (amended 2026-11-24): gc(verbose = TRUE) and gcinfo(TRUE) report lines
# match GNU R's format (with numbers normalized to placeholders), and the
# gcinfo switch returns its previous state.

invisible(mem.maxVSize(Inf))

normalize <- function(lines) {
  lines <- sub(
    "^Garbage collection [0-9]+ = [0-9]+\\+[0-9]+\\+[0-9]+ \\(level [0-2]\\) \\.\\.\\. $",
    "Garbage collection N = a+b+c (level L) ... ",
    lines
  )
  lines <- sub(
    "^[0-9]+\\.[0-9]+ Mbytes of cons cells used \\([0-9]+%\\)$",
    "X.Y Mbytes of cons cells used (P%)",
    lines
  )
  lines <- sub(
    "^[0-9]+\\.[0-9]+ Mbytes of vectors used \\([0-9]+%\\)$",
    "X.Y Mbytes of vectors used (P%)",
    lines
  )
  lines
}

# 1. gc(verbose = TRUE) prints one 3-line report.
tmp1 <- tempfile()
con1 <- file(tmp1, open = "wt")
sink(con1, type = "message")
invisible(gc(verbose = TRUE))
sink(type = "message")
close(con1)
lines1 <- readLines(tmp1)
cat("verbose_len:", length(lines1), "\n")
writeLines(normalize(lines1))

# 2. gcinfo(TRUE) returns old state and reports on automatic collections.
tmp2 <- tempfile()
con2 <- file(tmp2, open = "wt")
sink(con2, type = "message")
old1 <- gcinfo(TRUE)
old_na <- gcinfo(NA)
v_trig <- gc(verbose = FALSE)["Vcells", "gc trigger"]
chunk <- 1e6L
n_chunks <- as.integer(ceiling(v_trig * 1.5 / chunk))
for (i in seq_len(n_chunks)) {
  z <- numeric(chunk)
  rm(z)
}
old2 <- gcinfo(FALSE)
old3 <- gcinfo(FALSE)
sink(type = "message")
close(con2)
lines2 <- readLines(tmp2)
cat("switches:", old1, old_na, old2, old3, "\n")
cat("gcinfo_nonempty:", length(lines2) >= 3L && length(lines2) %% 3L == 0L, "\n")
# Every 3-line block has the exact normalized format.
blocks <- unique(normalize(lines2))
writeLines(blocks)
