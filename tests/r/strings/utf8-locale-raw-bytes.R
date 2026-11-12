# In a UTF-8 locale, cat, writeLines and message write a string's bytes
# when they are not UTF-8 (latin1 strings are translated), and private-use
# characters stay as they are.
x <- "\xe9t\xe9"
u <- iconv(intToUtf8(233), "UTF-8", "latin1")
y <- "a\xff"
Encoding(y) <- "UTF-8"
b <- "\xff\xfe"
Encoding(b) <- "bytes"
cat(x, u, y, b, "\n")
writeLines(c(x, u, y, b))
writeLines(c(x, u, y), useBytes = TRUE)
writeLines("a", sep = x)
cat("\n")
cat(x, fill = 4)
cat(x, u, x, fill = 6)
message(x)
print(c(x, u, y, b))
print(noquote(c(x, u, y, b)))
f <- tempfile()
writeLines(c(x, u), f)
print(readBin(f, "raw", 20))
p <- "a\uf7e9b"
print(p)
cat(p, "\n")
writeLines(p)
print(charToRaw(p))
dput(p)
print(c(NA, 1), na.print = p)
