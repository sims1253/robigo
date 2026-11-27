# 1e6 strings split by strsplit() (grep/strsplit.rs: a vector and its pieces per element): it collects inside its loop (E4; moved from mem-report by the strsplit/gsub/regexpr fill PR)
x <- strsplit(paste0("a,", 1:1000000), ",", fixed = TRUE)
cat(length(x), x[[1000000]], "\n")
