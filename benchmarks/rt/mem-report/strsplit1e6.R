# E4 report-only: strsplit (grep/strsplit.rs: a vector and its pieces per element); cured by: strsplit/gsub/regexpr's fill PR (B)
x <- strsplit(paste0("a,", 1:1000000), ",", fixed = TRUE)
cat(length(x), x[[1000000]], "\n")
