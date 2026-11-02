# memCompress/memDecompress (R: connections.c:do_memCompress,
# do_memDecompress): the wrapper's type codes, the zlib format for
# "gzip", bzip2 through libbz2, and "unknown" detection
# (comp_type_from_memory, with the zlib prefix 78 9c).
raw <- charToRaw("hello memDecompress")
print(memDecompress(memCompress(raw, "bzip2"), "bzip2"))
print(memDecompress(memCompress(raw, "gzip"), "gzip"))
print(memDecompress(raw, "none"))
print(memDecompress(memCompress(charToRaw("abc"), "bzip2"), "unknown", asChar = TRUE))
print(memDecompress(memCompress(charToRaw("whee"), "gzip"), "unknown", asChar = TRUE))
w <- 0
v <- withCallingHandlers(
  memDecompress(memCompress(charToRaw("plain"), "none"), "unknown", asChar = TRUE),
  warning = function(x) { w <<- 1; invokeRestart("muffleWarning") })
cat(v, w, "\n")
# the zlib format itself (78 9c) is also gzip-compressed data
print(memDecompress(memCompress(charToRaw("zlibfmt"), "gzip"), "unknown", asChar = TRUE))
# zlib/libdeflate stop after the first stream: concatenated members
# and trailing bytes return only the first member's bytes
gz2 <- c(memCompress(charToRaw("first"), "gzip"), memCompress(charToRaw("second"), "gzip"))
print(memDecompress(gz2, "gzip", asChar = TRUE))
bz2 <- c(memCompress(charToRaw("one"), "bzip2"), memCompress(charToRaw("two"), "bzip2"))
print(memDecompress(bz2, "bzip2", asChar = TRUE))
gzt <- c(memCompress(charToRaw("lead"), "gzip"), as.raw(c(1, 2, 3)))
print(memDecompress(gzt, "gzip", asChar = TRUE))
# lzop data is an error in R itself
lz <- tempfile()
writeBin(as.raw(c(0x89, 0x4c, 0x5a, 0x4f, 0x00, 0x0d, 0x0a)), lz)
print(tryCatch(memDecompress(readBin(lz, "raw", 99), "unknown"), error = function(e) "lzop-E"))
# malformed and truncated data: R's statuses verbatim
print(tryCatch(memDecompress(charToRaw("bad"), "gzip"), error = function(e) conditionMessage(e)))
print(tryCatch(memDecompress(charToRaw("bad"), "bzip2"), error = function(e) conditionMessage(e)))
tg <- memCompress(charToRaw("trunc"), "gzip")[1:6]
print(tryCatch(memDecompress(tg, "gzip"), error = function(e) conditionMessage(e)))
tbg <- memCompress(charToRaw("trunc"), "bzip2")[1:8]
print(tryCatch(memDecompress(tbg, "bzip2"), error = function(e) conditionMessage(e)))
