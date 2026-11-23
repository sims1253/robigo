# R: util.c:do_enc2 -- in a UTF-8 locale both enc2utf8 and enc2native
# re-mark native strings UTF-8 and translate latin1 through CP1252.
hex <- function(s) paste(sprintf("%02x", as.integer(charToRaw(s))), collapse = " ")
marks <- function(x) paste(Encoding(x), collapse = ",")

x <- c("\u00e9", "\303\251", "\xc3\xa9", "a", NA)
cat("input ", marks(x), "\n")
cat("utf8  ", marks(enc2utf8(x)), "\n")
cat("native", marks(enc2native(x)), "\n")
cat("bytes ", hex(enc2utf8(x)[1]), "|", hex(enc2utf8(x)[2]), "|", hex(enc2utf8(x)[3]), "\n")

y <- c("caf\u00e9", "caf\303\251")
Encoding(y[2]) <- "latin1"
u <- enc2utf8(y)
cat("latin1", marks(u), hex(u[1]), "|", hex(u[2]), "\n")
n <- enc2native(y)
cat("natlat", marks(n), hex(n[1]), "|", hex(n[2]), "\n")

for (byte in c(0x80, 0x81, 0x91, 0x9F, 0xE9)) {
    s <- rawToChar(as.raw(byte))
    Encoding(s) <- "latin1"
    e <- enc2utf8(s)
    cat(sprintf("cp1252 %02x %s %s\n", byte, Encoding(e), hex(e)))
}

b <- c("abc", "\303\251")
Encoding(b[2]) <- "bytes"
cat("bytesmark", marks(b), "->", marks(enc2utf8(b)), "\n")

bad <- rawToChar(as.raw(c(0x80, 0x81)))
cat("invalid", Encoding(bad), "->", Encoding(enc2utf8(bad)), hex(enc2utf8(bad)), "\n")

z <- structure(c("\u00e9", "\303\251"), names = c("a", "b"))
cat("names", paste(names(enc2utf8(z)), collapse = ","), "|", paste(names(enc2native(z)), collapse = ","), "\n")

cat("identical", identical(x, enc2utf8(x)), identical(u, enc2native(u)), "\n")
