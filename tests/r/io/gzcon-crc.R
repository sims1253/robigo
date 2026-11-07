# gzcon reading: a bad crc trailer is reported on stderr ("crc error"), not
# an error, and the inflated data is still returned (connections.c's
# gzcon_read); a truncated trailer is reported the same way. One readBin
# call reads the whole stream (GNU R reports again on every later read).
fil <- tempfile("gzc")
zz <- gzcon(file(fil, "wb"))
writeLines(c("alpha", "beta"), zz)
close(zz)
b <- readBin(fil, "raw", 100)
n <- length(b)
bad <- b
bad[n - 7L] <- as.raw(bitwXor(as.integer(bad[n - 7L]), 1L))
writeBin(bad, fil)
zz <- gzcon(file(fil, "rb"))
print(rawToChar(readBin(zz, "raw", 1000)))
close(zz)
writeBin(b[seq_len(n - 6L)], fil)
zz <- gzcon(file(fil, "rb"))
print(rawToChar(readBin(zz, "raw", 1000)))
close(zz)
# a header with ORIG_NAME and a stream passed through untouched
writeBin(c(b[1:3], as.raw(0x08), b[5:10], charToRaw("name"), as.raw(0), b[-(1:10)]), fil)
zz <- gzcon(file(fil, "rb"))
print(readLines(zz))
close(zz)
unlink(fil)
