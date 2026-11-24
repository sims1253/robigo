# E4 report-only: sprintf (sprintf.rs's strings); cured by: format/formatC/sprintf's fill_strings PR (B)
x <- sprintf("s%d", 1:1000000)
cat(length(x), x[1000000], "\n")
