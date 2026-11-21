# readDCF (dcf.c's do_readDCF), through read.dcf: records, fields asked for
# or all of them, folding (trailing white space, continuation lines, the
# " ." line) and keep.white, comments, blank lines, CRLF, an incomplete last
# line, a connection already open, and the two errors.
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e))
# a matrix as its parts (robigo-rt does not print matrices yet)
show <- function(m) { print(dim(m)); print(dimnames(m)); print(as.vector(m)) }
d <- if (dir.exists("data")) "data" else "tests/rt/connections/data"
a <- file.path(d, "a.dcf")
x <- read.dcf(a)
show(x)
show(read.dcf(a, fields = c("Version", "Imports", "Nope")))
show(read.dcf(a, fields = "Description", keep.white = "Description"))
show(read.dcf(a, keep.white = c("Title", "Imports")))
show(.Internal(readDCF(file(a), NULL, NULL)))
con <- file(a, "r")
print(readLines(con, 1))
show(read.dcf(con, fields = "Package"))
close(con)
show(read.dcf(file.path(d, "crlf.dcf")))
show(read.dcf(file.path(d, "empty.txt")))
print(msg(read.dcf(file.path(d, "bad-cont.dcf"))))
print(msg(read.dcf(file.path(d, "bad-line.dcf"))))
