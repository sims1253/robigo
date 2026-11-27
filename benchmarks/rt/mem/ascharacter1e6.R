# 1e6 doubles as character (coerceVector to STRSXP, coerce/vector.rs: a CHARSXP per element): it collects inside its loop (E4; moved from mem-report by the coerce paths' fill_strings PR)
x <- as.character((1:1000000) / 3)
cat(length(x), x[1000000], "\n")
