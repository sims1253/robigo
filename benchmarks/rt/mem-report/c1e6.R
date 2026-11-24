# E4 report-only: c's string answer (bind.rs's string_elt per element, coercing the doubles); cured by: the coerce paths' and c/unlist's fill_strings PR (B, after the paste poll)
x <- c("a", (1:1000000) / 3)
cat(length(x), x[1000001], "\n")
