# 1e6 doubles coerced to character by c() (StringAnswer, bind.rs: a CHARSXP per element): it collects inside its loop (E4; moved from mem-report by the c/unlist fill_strings + fill_list PR)
x <- c("a", (1:1000000) / 3)
cat(length(x), x[1000001], "\n")
