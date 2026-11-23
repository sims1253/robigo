# Names that are R strings deparse as translateChar(name): isValidName
# sees the native bytes, and those bytes are printed (C locale).
invisible(Sys.setlocale("LC_CTYPE", "C"))
nm <- paste0("caf", intToUtf8(233))
l <- list(1, 2); names(l) <- c(nm, "b")
dput(l)
x <- deparse(l); print(x); print(charToRaw(x)); print(Encoding(x))
dput(l, control = NULL)
x <- quote(a$b); x[[3]] <- nm
print(deparse(x)); print(charToRaw(deparse(x)))
l2 <- list(1); names(l2) <- iconv(nm, "UTF-8", "latin1")
dput(l2)
dput(c(a = 1, `b c` = 2))
