# Outside a UTF-8 locale, readLines() and scan() on a connection they open
# convert its declared encoding to UTF-8 (R: connections.c:set_iconv's
# `useUTF8 = !utf8locale && con->UTF8out`); open() converts to the locale's.
invisible(Sys.setlocale("LC_CTYPE", "C"))
print(l10n_info()[["UTF-8"]])
tf <- tempfile()
writeBin(as.raw(c(0x63, 0x61, 0x66, 0xe9, 0x0a, 0x61, 0x62, 0x0a)), tf)
l <- readLines(file(tf, encoding = "latin1")); print(Encoding(l)); print(lapply(l, charToRaw))
x <- scan(file(tf, encoding = "latin1"), "", quiet = TRUE); print(Encoding(x)); print(lapply(x, charToRaw))
con <- file(tf, encoding = "latin1")
x <- scan(con, "", quiet = TRUE); print(summary(con)$mode); close(con)
con <- file(tf, encoding = "latin1"); open(con)
l <- suppressWarnings(readLines(con)); print(l); close(con)
r <- tryCatch(readLines(file(tf, encoding = "no-such-enc")), error = conditionMessage); print(r)
r <- tryCatch(scan(file(tf, encoding = "no-such-enc"), "", quiet = TRUE), error = conditionMessage); print(r)
r <- tryCatch(open(file(tf, encoding = "no-such-enc")), error = conditionMessage); print(r)
