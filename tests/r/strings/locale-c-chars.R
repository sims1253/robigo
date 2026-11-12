# GAP: character functions in a single-byte locale. Native strings count
# bytes; UTF-8 strings are translated first (<U+XXXX>), so case mapping
# leaves the escape alone, as.name() warns and iconv(to = "") fails.
invisible(Sys.setlocale("LC_CTYPE", "C"))
u <- intToUtf8(c(99, 97, 102, 233))
n <- rawToChar(as.raw(c(0x63, 0x61, 0x66, 0xc3, 0xa9)))
print(nchar(c(u, n))); print(nchar(c(u, n), "width"))
print(toupper(u)); print(casefold(u, upper = TRUE))
print(sprintf("%-8s|", n))
print(as.name(u)); print(make.names(u))
print(iconv(u, "UTF-8", ""))
