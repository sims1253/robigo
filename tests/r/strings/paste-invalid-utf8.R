# paste0 does not carry encodings yet: GNU R concatenates the bytes of
# UTF-8-marked inputs and keeps the UTF-8 mark, so the result of two
# strings with unpaired surrogate halves is still invalid UTF-8 and
# utf8ToInt gives NA. robigo's paste builds Rust strings, so the
# byte-as-char pieces re-encode and the mark is lost.
print(utf8ToInt(paste0("\uD83D", "\uDE00")))
print(nchar(paste0("\uD83D", "\uDE00"), type = "chars"))
