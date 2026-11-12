# parse(text =) keeps the locale's literal marks only when every text
# element's encoding is known (marked, or ASCII), and reads the text
# translated to the native encoding (source.c:do_parse).
raw <- rawToChar(as.raw(c(0x22, 0x63, 0x61, 0x66, 0xc3, 0xa9, 0x22)))
src <- raw; Encoding(src) <- "UTF-8"
print(Encoding(c(raw, src)))
print(Encoding(eval(parse(text = src)[[1]])))
print(Encoding(eval(parse(text = raw)[[1]])))
print(Encoding(eval(parse(text = raw, encoding = "UTF-8")[[1]])))
invisible(Sys.setlocale("LC_CTYPE", "C"))
r <- eval(parse(text = src)[[1]])
print(Encoding(r)); print(r)
invisible(Sys.setlocale("LC_CTYPE", "C.UTF-8"))
print(r)
