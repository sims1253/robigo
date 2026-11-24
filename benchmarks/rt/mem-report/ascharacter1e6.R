# E4 report-only: coerceVector to STRSXP (as.character, as.vector; coerce/vector.rs's shared coerce); cured by: the coerce paths' fill_strings PR (B, after the paste poll)
x <- as.character((1:1000000) / 3)
cat(length(x), x[1000000], "\n")
