# writeLines() writes strings translated to the native encoding
# (translateChar0), or their bytes with useBytes = TRUE (connections.c).
invisible(Sys.setlocale("LC_CTYPE", "C"))
u <- intToUtf8(c(99, 97, 102, 233))
l <- iconv(u, "UTF-8", "latin1")
n <- rawToChar(as.raw(c(0x63, 0x61, 0x66, 0xc3, 0xa9)))
b <- n; Encoding(b) <- "bytes"
writeLines(c(u, l, n, b, NA))
writeLines(c(u, l), useBytes = TRUE)
writeLines(c("a", "b"), sep = intToUtf8(0xe0)); cat("\n")
f <- tempfile(); writeLines(c(u, l), f); print(readBin(f, "raw", 100))
tc <- textConnection("o", "w"); writeLines(u, tc); close(tc); print(o)
invisible(Sys.setlocale("LC_CTYPE", "C.UTF-8"))
writeLines(c(u, n))
