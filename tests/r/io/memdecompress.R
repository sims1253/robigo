# Gap: memCompress/memDecompress (the .Internal pair behind them) are
# not implemented; several packages decompress in-memory payloads with
# them (found while testing cli's bzip2 sysdata path).
raw <- charToRaw("hello memDecompress")
print(memDecompress(memCompress(raw, "bzip2"), "bzip2"))
print(memDecompress(memCompress(raw, "gzip"), "gzip"))
print(memDecompress(raw, "none"))
print(memDecompress(memCompress(charToRaw("abc"), "bzip2"), "bzip2", asChar = TRUE))
