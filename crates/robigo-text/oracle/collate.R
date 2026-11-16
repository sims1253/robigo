# Records tests/data/collate.tsv: GNU R 4.6.2's Scollate (ICU 76.1's root
# collator, in the oracle image's C.UTF-8) over the strings
# tests/collate.rs generates, read from stdin (first line: ORDER PAIRS).
#
#   N  ORDER PAIRS strings bytes
#   O  order(x, method = "shell") of the first ORDER, 1-based
#   P  one of < = > a pair: the sign of Scollate(a, b) for each pair after them
#
# oracle/record.sh runs it. Never record with a GNU R other than 4.6.2.

stopifnot(R.version$major == "4", R.version$minor == "6.2", capabilities("ICU"))

con <- file("stdin")
l <- readLines(con, encoding = "UTF-8")
close(con)
k <- as.integer(strsplit(l[[1L]], " ")[[1L]])
x <- l[-1L]
stopifnot(length(x) == k[[1L]] + 2L * k[[2L]])
cat("N", k[[1L]], k[[2L]], length(x), sum(nchar(x, type = "bytes")), sep = "\t")
cat("\n")
xo <- x[seq_len(k[[1L]])]
cat("O\t", paste(order(xo, method = "shell"), collapse = ","), "\n", sep = "")
p <- x[-seq_len(k[[1L]])]
a <- p[c(TRUE, FALSE)]
b <- p[c(FALSE, TRUE)]
cat("P\t", paste(c("<", "=", ">")[(a > b) - (a < b) + 2L], collapse = ""), "\n", sep = "")
stopifnot(icuGetCollate() == "root")
