# Gap: xz (and zstd) memCompress/memDecompress need liblzma/libzstd,
# which robigo does not link; the internal errors "not implemented".
# R: connections.c:do_memDecompress case 4.
print(rawToChar(memDecompress(memCompress(charToRaw("xz me"), "xz"), "xz")))
