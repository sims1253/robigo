# do_cat: atomic vectors' elements separated by sep, numbers in their own
# format (7 digits), NA, symbols; NULL and empty objects skipped; no
# newline at the end unless fill or a sep holds one.
cat(1, 2, "a", TRUE, NA, 1.5, "\n")
cat(1e-20, 1e20, 123456.7, 1/3, -0, NA_real_, NaN, Inf, -Inf, "\n")
cat(NA, NA_integer_, NA_character_, TRUE, 3L, "\n")
cat(quote(sym), "\n")
cat(1+2i, NA_complex_, 1-1i, "\n")
cat("x", NULL, "y", NULL, "z\n")
cat("no newline")
cat("\n")
cat(1:10, "\n")
cat("é", "日本", "\n")
