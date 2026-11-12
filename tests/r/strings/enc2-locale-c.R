# enc2native() translates UTF-8 and latin1 strings to the native encoding
# outside a UTF-8 locale; enc2utf8() translates native strings from it,
# with <xx> for the bytes that fail (util.c:do_enc2).
invisible(Sys.setlocale("LC_CTYPE", "C"))
u <- intToUtf8(c(99, 97, 102, 233))
l <- iconv(u, "UTF-8", "latin1")
n <- rawToChar(as.raw(c(0x63, 0x61, 0x66, 0xc3, 0xa9)))
x <- enc2native(c(u, l, n, "a", NA)); print(Encoding(x)); print(x)
y <- enc2utf8(c(u, l, n, "a", NA)); print(Encoding(y)); print(nchar(y, "bytes"))
print(y)
b <- n; Encoding(b) <- "bytes"; print(identical(enc2native(b), b))
invisible(Sys.setlocale("LC_CTYPE", "C.UTF-8"))
print(y); print(Encoding(enc2native(c(u, l, n))))
