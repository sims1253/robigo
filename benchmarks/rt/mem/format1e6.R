# 1e6 doubles formatted by format() (do_format, format.rs: a CHARSXP per element): it collects inside its loop (E4; moved from mem-report by the format/formatC/sprintf fill_strings PR)
x <- format((1:1000000) / 7)
cat(length(x), x[1000000], "\n")
