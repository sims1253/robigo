# With useBytes = TRUE, PCRE matches bytes, not characters: caseless
# matching does not fold a multibyte UTF-8 letter (R: grep.c compiles
# without PCRE_UTF8).
txt <- "café été"
grep("ÉTÉ", txt, ignore.case = TRUE, perl = TRUE)
grep("ÉTÉ", txt, ignore.case = TRUE, perl = TRUE, useBytes = TRUE)
regexpr("ÉTÉ", txt, ignore.case = TRUE, perl = TRUE, useBytes = TRUE)
