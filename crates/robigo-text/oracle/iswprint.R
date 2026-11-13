# Prints, as inclusive hex ranges "first last", the code points glibc's
# iswprint() calls printable in the running locale (record.sh: C.UTF-8),
# as GNU R 4.6.2's EncodeString sees them through mbrtowc (native strings).
stopifnot(R.version$major == "4", R.version$minor == "6.2", l10n_info()$`UTF-8`)
cp <- c(0:0xd7ff, 0xe000:0x10ffff)
s <- intToUtf8(cp, multiple = TRUE)
Encoding(s) <- "unknown"
p <- .Internal(encodeString(s, NULL, "", 3L, TRUE)) == s
p[cp < 0x20 | cp == 0x5c] <- cp[cp < 0x20 | cp == 0x5c] == 0x5c  # escaped, but printable
p[cp == 0] <- FALSE
cp <- cp[p]
brk <- c(1L, which(diff(cp) != 1L) + 1L)
end <- c(brk[-1L] - 1L, length(cp))
cat(sprintf("%x %x\n", cp[brk], cp[end]), sep = "")
