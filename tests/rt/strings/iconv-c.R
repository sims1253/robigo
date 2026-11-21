# iconv in the C locale: "" is ASCII, and //TRANSLIT transliterates as
# glibc's C locale does
u <- "café"
print(iconv(u, "UTF-8", "ASCII//TRANSLIT")); print(iconv(u, "UTF-8", "", sub = "byte"))
print(iconv("abc", "", "UTF-8")); print(Encoding(iconv(u, "UTF-8", "")))
print(iconv("a\xe9", "", "UTF-8")); print(iconv("a\xe9", "", "UTF-8", sub = "byte"))
