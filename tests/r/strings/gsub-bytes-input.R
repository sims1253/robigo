# An input marked "bytes" makes sub()/gsub() match bytes and mark a changed
# result "bytes"; an unchanged one keeps its flag (R: grep.c:do_gsub
# haveBytesInput, mkBytesNew, markBytesOld). read.dcf(all = TRUE) relies on
# this for a DESCRIPTION file with non-ASCII fields.
x <- "Author: François"
b <- x; Encoding(b) <- "bytes"
s <- sub("[^:]*:[ ]*", "", b)
Encoding(s); charToRaw(s)
g <- gsub("o", "0", b)
Encoding(g); charToRaw(g)
u <- sub("zzz", "", b); Encoding(u)
m <- sub("Fran", "", c(b, "café")); Encoding(m); charToRaw(m[2])
a <- sub("Author: ", "", b); Encoding(a)
# paste() joins the bytes of "bytes" pieces untranslated and marks the
# element "bytes" (R: paste.c:do_paste use_Bytes); other elements stay text
p <- paste(c(b, "x"), "é"); Encoding(p); charToRaw(p[1]); charToRaw(p[2])
q <- paste(c(b, "x"), collapse = "\n"); Encoding(q); charToRaw(q)
paste0("a", "b", collapse = b)
Encoding(paste0("a", "b", collapse = b))
f <- tempfile()
writeLines(c("Package: x", paste0("Author: Romain François")), f, useBytes = TRUE)
d <- read.dcf(f, all = TRUE)
charToRaw(d$Author); Encoding(d$Author)
identical(unname(read.dcf(f)[1, ]), unname(unlist(d)))
unlink(f)
