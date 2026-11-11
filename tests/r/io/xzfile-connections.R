# R: connections.c:xzfile_open/xzfile_read/xzfile_write, do_gzfile,
# do_memCompress/do_memDecompress (xz). xzfile() writes one .xz stream per
# open (LZMA2 at the `compression` preset, extreme when negative, CRC32
# check); "a" appends another and the reader decodes the concatenation.
f <- tempfile()
con <- xzfile(f, "w")
writeLines(c("alpha", "beta"), con)
close(con)
con <- xzfile(f, "a", compression = -3)
writeLines("gamma", con)
close(con)
print(readLines(f))
print(readLines(xzfile(f)))
print(readLines(gzfile(f)))
con <- file(f); print(summary(con)$class); close(con)
con <- gzfile(f); print(summary(con)$class); close(con)
b <- readBin(f, "raw", 6L)
print(b)

# binary round trip
g <- tempfile()
con <- xzfile(g, "wb")
writeBin(as.raw(0:255), con)
close(con)
con <- xzfile(g, "rb")
print(identical(readBin(con, "raw", 1000L), as.raw(0:255)))
close(con)

# a file too short to be xz (liblzma: LZMA_BUF_ERROR at its end)
h <- tempfile()
writeLines("plain text", h)
print(readLines(xzfile(h)))
print(tryCatch(xzfile(f, compression = 10), error = conditionMessage))

# memCompress/memDecompress
x <- charToRaw(strrep("abc", 200))
y <- memCompress(x, "xz")
print(length(y))
print(y[1:6])
print(identical(memDecompress(y, "xz"), x))
print(identical(memDecompress(y, "unknown"), x))
print(rawToChar(memDecompress(memCompress(raw(0), "xz"), "xz")))
print(tryCatch(memDecompress(as.raw(c(0xfd, 0x37, 0x7a, 0x58, 0x5a, 0, 1, 2, 3)), "xz"),
               error = conditionMessage))
unlink(c(f, g, h))
