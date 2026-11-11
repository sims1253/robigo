# R: connections.c:xzfile_read warns at every read that ends in a decoder
# error, and liblzma's errors are sticky: a truncated stream gives
# LZMA_BUF_ERROR (10) at each read, corrupt data LZMA_DATA_ERROR and then
# LZMA_PROG_ERROR (11). Gap: robigo decodes the whole file at open and
# warns once.
f <- tempfile(); h <- tempfile()
con <- xzfile(f, "w"); writeLines(c("alpha", "beta"), con); close(con)
z <- readBin(f, "raw", file.size(f))
writeBin(z[seq_len(length(z) - 20L)], h)
print(readLines(xzfile(h)))
con <- xzfile(h, "rb"); print(readBin(con, "raw", 100)); print(readBin(con, "raw", 100)); close(con)
writeBin(c(z[1:30], as.raw(1:40)), h)
print(readLines(xzfile(h), warn = FALSE))
unlink(c(f, h))
