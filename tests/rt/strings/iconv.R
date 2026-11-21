# iconv (sysutils.c do_iconv) over the C library's iconv, sub included
p <- function(x) print(x)
e <- function(x) { p(x); p(Encoding(x)) }
l <- "caf\xe9"; Encoding(l) <- "latin1"
u <- "café"
e(iconv(l, "latin1", "UTF-8")); e(iconv(u, "UTF-8", "latin1")); e(iconv(u, "UTF-8", "latin1", mark = FALSE))
e(iconv(u, "", "UTF-8")); e(iconv(l, "", "UTF-8")); e(iconv(c("ab", NA, u), "UTF8", "utf8"))
p(iconv(u, "UTF-8", "ASCII")); p(iconv(c(a = u, b = "x"), "UTF-8", "ASCII", sub = "byte"))
p(iconv(u, "UTF-8", "ASCII", sub = "Unicode")); p(iconv("\U0001F600x", "UTF-8", "ASCII", sub = "Unicode"))
p(iconv(c(u, "中\U0001F600"), "UTF-8", "ASCII", sub = "c99")); p(iconv(u, "UTF-8", "ASCII", sub = "?"))
p(iconv(u, "UTF-8", "ASCII", sub = "")); p(iconv(u, "UTF-8", "ASCII", sub = "<bad>")); p(iconv(u, "UTF-8", "ASCII", sub = NA))
p(iconv("a\xffb", "UTF-8", "ASCII", sub = "byte")); p(iconv("a\xffb", "latin1", "ASCII", sub = "Unicode"))
p(iconv("a\xffb", "UTF-8", "UTF-8", sub = "byte")); p(iconv("a\xe9", "UTF-8", "latin1", sub = "byte"))
p(iconv(u, "UTF-8", "ASCII//TRANSLIT")); p(iconv(l, "latin1", "ASCII//TRANSLIT"))
p(iconv(u, "UTF-8", "UTF-16LE", toRaw = TRUE)); p(iconv(list(as.raw(c(0x63, 0xe9)), NULL), "latin1", "UTF-8"))
p(iconv(list(as.raw(c(0x61, 0x00, 0x62, 0x00))), "UTF-16LE", "UTF-8"))
p(iconv(list(as.raw(c(0x61, 0x00, 0x00, 0xd8))), "UTF-16LE", "UTF-8", sub = "byte"))
p(iconv(list(as.raw(0xe9), NULL), "latin1", "UTF-8", toRaw = TRUE)); p(iconv(list(as.raw(0xff)), "UTF-8", "ASCII", toRaw = TRUE))
p(iconv(c(NA, "x"), "UTF-8", "latin1", toRaw = TRUE)); p(iconv(character(0), "UTF-8", "latin1")); p(iconv(NULL))
p(iconv(1:3, "UTF-8", "latin1")); f <- factor(c("a", "b")); p(iconv(f, "", ""))
r <- function(x) print(tryCatch(x, error = conditionMessage))
r(iconv(u, "UTF-8", "NOT-AN-ENCODING")); r(iconv(u, "NOPE", "UTF-8")); r(iconv("a", c("a", "b"), "UTF-8"))
r(iconv("a", "UTF-8", 1)); r(.Internal(iconv("a", "UTF-8", "latin1", 1, TRUE, FALSE))); r(iconv("a", "UTF-8", "latin1", mark = NA))
r(iconv("a", "UTF-8", "latin1", toRaw = NA)); r(.Internal(iconv(1, "UTF-8", "latin1", NA_character_, TRUE, FALSE)))
r(iconv(list(1), "UTF-8", "latin1")); r(iconv("a", "UTF-8", "UTF-16LE"))
# a stateful target past the first buffer: GNU R doubles cbuff and starts
# the element again after the descriptor's reset (its shift sequence first)
j <- iconv(strrep("中", 5000), "UTF-8", "ISO-2022-JP", toRaw = TRUE)[[1]]
p(length(j)); p(j[1:8]); p(j[length(j) - 2:0])
k <- iconv(c(strrep("中", 5000), "中"), "UTF-8", "ISO-2022-JP", toRaw = TRUE); p(lengths(k)); p(k[[2]])
p(nchar(iconv(strrep("é", 9000), "UTF-8", "ASCII", sub = "byte")))
# mkCharLenCE's embedded-NUL error comes at its element
r(iconv(list(as.raw(0), 1), "UTF-8", "UTF-8"))
