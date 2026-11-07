# gzcon(): a gzip wrapper over another binary connection (connections.c's
# gzconn) — writeLines through it, readLines back, readBin of the raw file,
# and the non-compressed passthrough with allowNonCompressed = TRUE.
fil <- tempfile("gzc")
zz <- gzcon(file(fil, "wb"))
writeLines(c("TITLE extra line", "2 3 5 7", "", "11 13 17"), zz, sep = "\n")
close(zz)
zz <- gzcon(file(fil, "rb"))
print(readLines(zz))
close(zz)
# the same file through gzfile
print(readLines(gzfile(fil)))
# readBin through gzcon
zz <- gzcon(file(fil, "rb"))
print(readBin(zz, "character", n = 2))
close(zz)
# an uncompressed stream: refused without allowNonCompressed, passed through
# with it
fil2 <- tempfile("plain")
cat("plain text\n", file = fil2)
con <- gzcon(file(fil2, "rb"))
print(try(readLines(con), silent = TRUE))
close(con)
con <- gzcon(file(fil2, "rb"), allowNonCompressed = TRUE)
print(readLines(con))
close(con)
# wrapping a gzcon warns and returns it unchanged
zz <- gzcon(file(fil, "rb"))
print(inherits(zz2 <- gzcon(zz), "gzcon"))
close(zz)
unlink(c(fil, fil2))
