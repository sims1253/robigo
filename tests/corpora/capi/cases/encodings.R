dyn.load("encodings.so")
err <- function(expr) tryCatch(expr, error = function(e) cat("Error:", conditionMessage(e), "\n"))
NATIVE <- 0L; UTF8 <- 1L; LATIN1 <- 2L; BYTES <- 3L; SYMBOL <- 5L; ANY <- 99L

# rows: native, UTF-8, latin1, bytes; columns: getCharCE, charIsASCII,
# charIsUTF8, charIsLatin1 of "\xc3\xa9", then of "abc"
print(.Call("capi_enc_marks"))
print(.Call("capi_enc_identity"))

# what R sees of a CHARSXP made in each encoding
e <- as.raw(c(0x63, 0x61, 0x66, 0xc3, 0xa9))
for (enc in c(NATIVE, UTF8, LATIN1, BYTES)) {
  x <- .Call("capi_enc_mk", e, enc)
  print(Encoding(x)); print(charToRaw(x))
}
print(Encoding(.Call("capi_enc_mk", charToRaw("abc"), BYTES)))

# GNU R's refusals
err(.Call("capi_enc_mk", e, 7L))
err(.Call("capi_enc_mk", as.raw(c(0x61, 0, 0x62)), UTF8))
err(.Call("capi_enc_mk", as.raw(c(0x61, 0, 0x62)), 7L))
err(.Call("capi_enc_mk", as.raw(c(0x61, 0, 0x62)), BYTES))
err(.Call("capi_enc_mk", as.raw(c(0x61, 0, 0x62)), LATIN1))
err(.Call("capi_enc_mk", as.raw(c(0xc3, 0xa9, 0, 0x0a)), UTF8))
err(.Call("capi_enc_mk", as.raw(c(0xe9, 0)), LATIN1))
err(.Call("capi_enc_mk", as.raw(c(0xe9, 0)), BYTES))
print(.Call("capi_enc_symbol_any", SYMBOL))
print(.Call("capi_enc_symbol_any", ANY))
err(.Call("capi_enc_symbol_new", SYMBOL))
err(.Call("capi_enc_symbol_new", ANY))
print(Encoding(.Call("capi_enc_mk", charToRaw("ascii"), SYMBOL)))

# translation: latin1 is Windows-1252 (0x80 is the euro sign, 0x81 has no
# character), bytes cannot be translated
l1 <- .Call("capi_enc_mk", as.raw(c(0x63, 0x61, 0x66, 0xe9, 0x20, 0x80, 0x81)), LATIN1)
for (how in 0:2) err(print(.Call("capi_enc_translate", l1, how)))
by <- .Call("capi_enc_mk", as.raw(c(0x61, 0xff, 0x62)), BYTES)
err(.Call("capi_enc_translate", by, 0L))
err(.Call("capi_enc_translate", by, 1L))
err(.Call("capi_enc_translate", by, 2L))
u <- .Call("capi_enc_mk", e, UTF8)
n <- .Call("capi_enc_mk", e, NATIVE)
print(.Call("capi_enc_translate_same", u))
print(.Call("capi_enc_translate_same", n))
print(.Call("capi_enc_translate_same", "plain"))
print(.Call("capi_enc_translate_same", NA_character_))
err(.Call("capi_enc_not_charsxp", 1L, 0L))
err(.Call("capi_enc_not_charsxp", list(), 1L))
err(.Call("capi_enc_not_charsxp", NULL, 2L))

# reEnc(x, from, to, subst)
euro <- as.raw(c(0x41, 0xe2, 0x82, 0xac, 0x42))
for (subst in 0:3) print(.Call("capi_enc_reenc", euro, UTF8, LATIN1, subst))
print(.Call("capi_enc_reenc", as.raw(c(0x63, 0xe9, 0x80)), LATIN1, UTF8, 0L))
print(.Call("capi_enc_reenc", euro, UTF8, NATIVE, 1L))
print(.Call("capi_enc_reenc", euro, NATIVE, UTF8, 1L))
print(.Call("capi_enc_reenc", euro, BYTES, UTF8, 1L))
print(.Call("capi_enc_reenc", euro, UTF8, BYTES, 1L))
print(.Call("capi_enc_reenc", euro, UTF8, ANY, 1L))
print(.Call("capi_enc_reenc", charToRaw("abc"), LATIN1, UTF8, 1L))
print(.Call("capi_enc_reenc", charToRaw("abc"), SYMBOL, UTF8, 1L))
print(.Call("capi_enc_reenc", charToRaw("abc"), SYMBOL, LATIN1, 1L))
print(.Call("capi_enc_reenc", as.raw(c(0x41, 0xff, 0x42)), UTF8, LATIN1, 1L))

# ucstoutf8
for (cp in c(0x41, 0xe9, 0x20ac, 0x1f600, 0)) print(.Call("capi_enc_ucs", cp))
err(.Call("capi_enc_ucs", 0xd800))
err(.Call("capi_enc_ucs", 0x110000))

# R_nchar(x, type, allowNA, msg_name): Bytes 0, Chars 1, Width 2
wide <- .Call("capi_enc_mk", charToRaw("日本"), UTF8)
bad <- .Call("capi_enc_mk", as.raw(c(0x61, 0xff)), UTF8)
badn <- .Call("capi_enc_mk", as.raw(c(0x61, 0xff)), NATIVE)
for (x in list(l1, u, n, wide, by, bad, badn, "plain"))
  for (type in 0:2) print(c(type, .Call("capi_enc_nchar", x, type, FALSE, NULL),
                            .Call("capi_enc_nchar", x, type, TRUE, NULL)))
err(.Call("capi_enc_nchar", by, 1L, FALSE, "element 1"))
err(.Call("capi_enc_nchar", by, 2L, FALSE, "element 1"))
err(.Call("capi_enc_nchar", bad, 1L, FALSE, "element 2"))
err(.Call("capi_enc_nchar", badn, 2L, FALSE, "element 3"))
print(.Call("capi_enc_nchar", NA_character_, 1L, FALSE, NULL))

# symbols
print(.Call("capi_enc_install", l1))
print(.Call("capi_enc_install", u))
print(Encoding(.Call("capi_enc_install", .Call("capi_enc_mk", charToRaw("\u00e9t\u00e9_sym"), UTF8))[[3]]))
print(Encoding(.Call("capi_enc_install", .Call("capi_enc_mk", charToRaw("\u00e9t\u00e9_nat"), NATIVE))[[3]]))

# the locale data, here and in the C locale
print(.Call("capi_enc_locale"))
invisible(Sys.setlocale("LC_CTYPE", "C"))
print(.Call("capi_enc_locale"))
print(.Call("capi_enc_translate", u, 0L))
print(.Call("capi_enc_translate", u, 1L))
print(.Call("capi_enc_translate", l1, 0L))
print(.Call("capi_enc_translate", n, 1L))
err(.Call("capi_enc_translate", u, 2L))
print(.Call("capi_enc_translate_same", u))
print(.Call("capi_enc_reenc", euro, UTF8, NATIVE, 1L))
print(.Call("capi_enc_marks")[, 7:8])
for (type in 1:2) print(.Call("capi_enc_nchar", n, type, TRUE, NULL))
invisible(Sys.setlocale("LC_CTYPE", "C.UTF-8"))
print(.Call("capi_enc_locale"))
print(.Call("capi_enc_translate", u, 0L))
