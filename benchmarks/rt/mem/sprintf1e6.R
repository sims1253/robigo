# 1e6 integers formatted by sprintf() (do_sprintf, sprintf.rs: a CHARSXP per element): it collects inside its loop (E4; moved from mem-report by the format/formatC/sprintf fill_strings PR)
x <- sprintf("s%d", 1:1000000)
cat(length(x), x[1000000], "\n")
