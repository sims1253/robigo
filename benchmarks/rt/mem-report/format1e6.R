# E4 report-only: format (robigo-rt format.rs's do_format); cured by: format/formatC/sprintf's fill_strings PR (B)
x <- format((1:1000000) / 7)
cat(length(x), x[1000000], "\n")
