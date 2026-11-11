# gzfile()/bzfile() connections are text unless the mode has a "b", and a
# text open checks the encoding (R: connections.c:gzfile_open, bzfile_open).
f <- tempfile(fileext = ".gz")
writeLines(c("a", "b"), gzfile(f))
con <- gzfile(f, "r"); summary(con)$text
r <- try(readBin(con, "raw", 3)); close(con)
con <- gzfile(f, "rb"); summary(con)$text; readBin(con, "raw", 3); close(con)
con <- gzfile(f, "rt"); summary(con)$text; readLines(con); close(con)
con <- gzfile(f, "rb"); readLines(con); close(con)
con <- gzfile(f, "r"); pushBack("x", con); readLines(con); close(con)
con <- gzfile(f); open(con, "r"); summary(con)$text; close(con)
con <- gzfile(f, "w"); summary(con)$text
r <- try(writeBin(as.raw(1), con)); writeLines("c", con); close(con)
readLines(f)
con <- gzfile(f, "wb"); writeBin(charToRaw("d\n"), con); close(con)
readLines(f)
b <- tempfile(fileext = ".bz2")
writeLines("e", bzfile(b))
con <- bzfile(b, "r"); summary(con)$text; readLines(con); close(con)
con <- bzfile(b, "rb"); summary(con)$text; close(con)
# a text open with an encoding iconv cannot convert fails
nopen <- nrow(showConnections())
con <- gzfile(f, encoding = "no-such-enc")
r <- try(open(con, "r"))
con <- gzfile(f, encoding = "no-such-enc")
open(con, "rb"); summary(con)$text; close(con)
nrow(showConnections()) == nopen
con <- gzfile(f, "r", encoding = "latin1"); readLines(con); close(con)
unlink(c(f, b))
