# Conversions GNU R leaves to the C library's iconv.
x <- c(a = "café", b = "€5", c = "└─", d = NA, e = "abc", f = "ß")
print(iconv(x, "UTF-8", "ASCII//TRANSLIT"))
print(iconv(x, "", "ASCII//TRANSLIT"))
print(iconv(x, "UTF-8", "ASCII//TRANSLIT", sub = "!"))
print(iconv(x, "UTF-8", "ASCII//IGNORE"))
print(iconv(x, "UTF-8", "ASCII//IGNORE", sub = "byte"))
print(iconv(x, "UTF-8", "ISO-8859-1//TRANSLIT"))
print(Encoding(iconv("café", "UTF-8", "ASCII//TRANSLIT")))

# Other encodings, both ways.
print(iconv("\xb1", "ISO-8859-2", "UTF-8"))
print(Encoding(iconv("\xb1", "ISO-8859-2", "UTF-8")))
print(iconv("\x80", "CP1252", "UTF-8"))
print(iconv("ą", "utf8", "ISO-8859-2", toRaw = TRUE))
print(iconv("ab", "UTF-8", "UTF-16LE", toRaw = TRUE))
print(iconv(list(as.raw(c(0x61, 0, 0x62, 0)), NULL), "UTF-16LE", "UTF-8"))
print(iconv(list(as.raw(c(0x61, 0)), NULL), "UTF-16LE", "UTF-8", toRaw = TRUE))
print(iconv("ab", "ASCII", "ISO-8859-2"))

# Substitution, including characters outside the BMP.
s <- "a€\U0001f600b"
print(iconv(s, "UTF-8", "ISO-8859-2"))
print(iconv(s, "UTF-8", "ISO-8859-2", toRaw = TRUE))
print(iconv(s, "UTF-8", "ISO-8859-2", sub = "Unicode"))
print(iconv(s, "UTF-8", "ISO-8859-2", sub = "c99"))
print(iconv(s, "UTF-8", "ISO-8859-2", sub = "byte"))
print(iconv(s, "UTF-8", "ISO-8859-2", sub = "<?>"))
print(iconv(s, "UTF-8", "ISO-8859-2", sub = ""))
print(iconv(s, "", "ISO-8859-2", sub = "Unicode"))
print(iconv("\xe9", "ISO-8859-3", "ISO-8859-5", sub = "Unicode"))

# Input in units of more than a byte.
print(iconv(list(as.raw(c(0xac, 0x20, 0x61, 0))), "UTF-16LE", "ASCII", sub = "?"))
print(iconv(list(as.raw(c(0xac, 0x20, 0, 0, 0x61, 0, 0, 0))), "UTF-32LE", "ASCII", sub = "byte"))
print(iconv(list(as.raw(c(0xac, 0x20, 0x61, 0))), "UCS-2LE", "ASCII", sub = "byte"))

# Invalid and incomplete input.
print(iconv("a\xffb", "UTF-8", "ISO-8859-2"))
print(iconv("a\xffb", "UTF-8", "ISO-8859-2", sub = "byte"))
print(iconv("a\xe2\x82", "UTF-8", "ISO-8859-2", sub = "?"))

# The buffer grows.
print(length(iconv(strrep("é", 5000), "UTF-8", "UTF-16LE", toRaw = TRUE)[[1]]))
print(nchar(iconv(strrep("€", 5000), "UTF-8", "ASCII//TRANSLIT")))
print(nchar(iconv(strrep("€", 500), "UTF-8", "ISO-8859-2", sub = "Unicode")))

# A converter is only opened for an element that needs it.
print(iconv(NA_character_, "UTF-8", "no-such-encoding"))
print(iconv(character(), "UTF-8", "no-such-encoding"))
print(tryCatch(iconv("a", "UTF-8", "no-such-encoding"), error = conditionMessage))
print(tryCatch(iconv("a", "no-such-encoding", "UTF-8"), error = conditionMessage))
print(tryCatch(iconv("é", "", "no-such-encoding"), error = conditionMessage))
print(tryCatch(iconv(enc2native("\xe9"), "latin1", "no-such-encoding"), error = conditionMessage))
l <- "\xe9"
Encoding(l) <- "latin1"
print(tryCatch(iconv(l, "", "no-such-encoding"), error = conditionMessage))
print(tryCatch(iconv(list(1), "UTF-16LE", "UTF-8"), error = conditionMessage))

# The locale decides what "" means and what l10n_info() reports.
print(unlist(l10n_info()))
old <- Sys.getlocale("LC_CTYPE")
invisible(Sys.setlocale("LC_CTYPE", "C"))
print(unlist(l10n_info()))
print(iconv(c("café", "€"), "UTF-8", "ASCII//TRANSLIT"))
print(iconv(list(as.raw(c(0x61, 0xff))), "", "ISO-8859-2", sub = "byte"))
invisible(Sys.setlocale("LC_CTYPE", old))
print(unlist(l10n_info()))

# Which names mean UTF-8, and what the result is marked as.
print(iconv("€", "utf8", "ISO-8859-2", sub = "Unicode"))
print(iconv("€", "UTF8", "ISO-8859-2", sub = "c99"))
print(iconv("€", "utf-8", "ISO-8859-2", sub = "Unicode"))
print(tryCatch(iconv("a", "utf8", "no-such-encoding"), error = conditionMessage))
print(tryCatch(iconv("a", "no-such-encoding", "Utf8"), error = conditionMessage))
print(Encoding(iconv("€", "UTF-8", "CP1252")))
print(Encoding(iconv("€", "UTF-8", "cp1252", mark = FALSE)))
print(iconv("€", "UTF-8", "CP1252", toRaw = TRUE))
print(Encoding(iconv("\xb1", "ISO-8859-2", "")))
print(Encoding(iconv("\xb1", "ISO-8859-2", "utf8")))
print(Encoding(iconv("ą", "UTF-8", "ISO-8859-2")))

# A mark overrides from = "", and with it the locale's UTF-8.
l <- "\xc3\xa9"
Encoding(l) <- "latin1"
print(iconv(l, "", "ISO-8859-5", sub = "Unicode"))
print(iconv(l, "", "ISO-8859-5", sub = "c99"))

# Each element starts from the initial shift state, written to its output.
print(iconv(c("あ", "a"), "UTF-8", "ISO-2022-JP", toRaw = TRUE))
print(iconv(c("あ", "あ"), "UTF-8", "ISO-2022-JP", toRaw = TRUE))
print(iconv(c("a", "b"), "UTF-8", "UTF-16", toRaw = TRUE))
