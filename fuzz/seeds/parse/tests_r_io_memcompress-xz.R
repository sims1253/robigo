# xz memCompress/memDecompress round trip (liblzma, as GNU R).
# R: connections.c:do_memCompress, do_memDecompress case 4.
print(rawToChar(memDecompress(memCompress(charToRaw("xz me"), "xz"), "xz")))
