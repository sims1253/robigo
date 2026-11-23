# strsplit(useBytes = TRUE) splits a string that is not valid UTF-8 at
# its bytes, in both the TRE and the PCRE branch (R: grep.c:do_strsplit).
x <- "\xf1\xa1_\xc5\xa2"
r <- strsplit(x, "_", useBytes = TRUE, perl = TRUE)
lapply(r, function(s) lapply(s, charToRaw))
Encoding(r[[1]])
identical(r, list(c("\xf1\xa1", "\xc5\xa2")))
r2 <- strsplit(x, "_", useBytes = TRUE)
lapply(r2, function(s) lapply(s, charToRaw))
identical(r2, list(c("\xf1\xa1", "\xc5\xa2")))
