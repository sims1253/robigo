# Index labels use the printed length, including max.print's +1 exception.
for (limit in c(1L, 5L, 9L)) {
    options(max.print = limit)
    print(1:10)
}
options(max.print = 5)
for (x in list(rep(TRUE, 10), 1:10, as.double(1:10), as.complex(1:10),
               as.character(1:10), as.raw(1:10))) print(x)
# Keep byte-window encoding and escaping intact when output is truncated.
options(width = 20)
for (encoding in c("unknown", "latin1", "bytes", "UTF-8")) {
    x <- if (encoding == "bytes") "ASCII" else rawToChar(as.raw(196))
    Encoding(x) <- encoding
    print(rep(x, 10))
}
options(max.print = 10)
print(1:100)
