x <- "\304"
print(charToRaw(x))
print(Encoding(x))
print(validUTF8(x))
print(validEnc(x))
print(nchar(x, type = "bytes"))
print(x)
y <- rawToChar(as.raw(c(0, 65, 127, 128, 196, 255)), multiple = TRUE)
print(y)
print(Encoding(y))
print(lapply(y, charToRaw))
print(validUTF8(y))
print(validEnc(y))
for (encoding in c("unknown", "latin1", "bytes", "UTF-8")) {
    z <- x
    Encoding(z) <- encoding
    print(Encoding(z))
    print(charToRaw(z))
    print(validUTF8(z))
    print(validEnc(z))
    copy <- unserialize(serialize(z, NULL))
    print(Encoding(copy))
    print(charToRaw(copy))
}
print(charToRaw(rawToChar(as.raw(c(195, 169, 0, 0)))))
print(Encoding(rawToChar(as.raw(c(195, 169)))))
print(Encoding(c("ASCII", NA_character_)))
for (b in 1:255) stopifnot(identical(charToRaw(rawToChar(as.raw(b))), as.raw(b)))
