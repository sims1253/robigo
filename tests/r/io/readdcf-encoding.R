# read.dcf() returns fields as dcf.c's mkChar makes them: in the native
# encoding, so Encoding() is "unknown" even for UTF-8 text, and bytes
# invalid in UTF-8 (a latin1 DESCRIPTION) are kept as they are.
f <- tempfile()
writeLines(c("Package: foo", "Title: Café au lait", "Encoding: UTF-8",
             "Author: Jörg", "Description: Line one", "  naïve continued."),
           f, useBytes = TRUE)
d <- read.dcf(f)
d
Encoding(d)
Encoding(colnames(d))
d2 <- read.dcf(f, keep.white = "Description")
d2[, "Description"]
Encoding(d2)
d5 <- read.dcf(f, fields = c("Title", "Missing"))
d5
Encoding(d5)
f2 <- tempfile()
writeBin(c(charToRaw("Package: foo\nTitle: Caf"), as.raw(0xe9),
           charToRaw("\nEncoding: latin1\n")), f2)
d3 <- read.dcf(f2)
Encoding(d3)
d3[, "Title"] == "Caf\xe9"
charToRaw(d3[, "Title"])
nchar(d3[, "Title"], "bytes")
d4 <- read.dcf(f2, all = TRUE)
Encoding(d4$Title)
charToRaw(enc2utf8(iconv(d4$Title, "latin1", "UTF-8")))
unlink(c(f, f2))
