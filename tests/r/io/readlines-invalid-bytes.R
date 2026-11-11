# readLines() keeps a line's bytes as they are, even where they are not
# valid UTF-8 (R: connections.c:do_readLines makes a native CHARSXP).
tf <- tempfile()
writeBin(as.raw(c(0x78, 0xab, 0x7a, 0x20, 0xe7, 0x61, 0x0a)), tf)
b <- readLines(tf)
nchar(b, "bytes")
charToRaw(b)
