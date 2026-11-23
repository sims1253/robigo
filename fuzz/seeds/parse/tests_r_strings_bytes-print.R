# Strings marked "bytes" print their non-ASCII bytes as \xHH in every
# locale, and strings not valid in the locale's encoding escape the bytes.
x <- rawToChar(as.raw(c(0x61, 0xff, 0x62, 0x5c)))
Encoding(x) <- "bytes"
print(x); print(x, quote = FALSE); print(c(x, "abcdefghijk"))
print(format(x)); print(nchar(format(x))); print(format(c(x, "a")))
print(encodeString(x)); print(encodeString(x, quote = '"'))
cat(x, "\n"); dput(x)
q <- rawToChar(as.raw(c(0x61, 0x22, 0xff, 0x62)))
Encoding(q) <- "bytes"
print(q); print(encodeString(q, quote = '"'))
l <- rawToChar(as.raw(c(0x63, 0x61, 0x66, 0xe9, 0x80)))
Encoding(l) <- "latin1"
cat(l, "\n"); print(format(l)); print(Encoding(format(l)))
invisible(Sys.setlocale("LC_CTYPE", "C"))
print(x); print(format(x)); print(encodeString(x)); cat(x, "\n"); dput(x)
y <- rawToChar(as.raw(c(0x61, 0xff, 0x62)))
print(y); print(encodeString(y)); dput(y)
print(l); print(format(l)); cat(l, "\n")
invisible(Sys.setlocale("LC_CTYPE", "C.UTF-8"))
print(l); print(x)
