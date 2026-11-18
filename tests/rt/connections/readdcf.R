# readDCF (dcf.c's do_readDCF), through read.dcf: records, fields asked for
# or all of them, folding (trailing white space, continuation lines, the
# " ." line) and keep.white, comments, blank lines, CRLF, an incomplete last
# line, a connection already open, and the two errors.
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e))
d <- if (dir.exists("data")) "data" else "tests/rt/connections/data"
a <- file.path(d, "a.dcf")
x <- read.dcf(a)
print(dim(x))
print(colnames(x))
print(x)
print(read.dcf(a, fields = c("Version", "Imports", "Nope")))
print(read.dcf(a, fields = "Description", keep.white = "Description"))
print(read.dcf(a, keep.white = c("Title", "Imports")))
print(.Internal(readDCF(file(a), NULL, NULL)))
con <- file(a, "r")
print(readLines(con, 1))
print(read.dcf(con, fields = "Package"))
close(con)
print(read.dcf(file.path(d, "crlf.dcf")))
print(read.dcf(file.path(d, "empty.txt")))
print(msg(read.dcf(file.path(d, "bad-cont.dcf"))))
print(msg(read.dcf(file.path(d, "bad-line.dcf"))))
