# The parser marks string literals by the locale at parse time: UTF-8 in
# a UTF-8 locale, native otherwise, and UTF-8 for \u escapes always.
lit <- paste0('"caf', "\\", 'u00e9"')
oct <- paste0('"caf', "\\", '303', "\\", '251"')
print(Encoding("café"))
print(Encoding(eval(parse(text = lit)[[1]])))
print(Encoding(eval(parse(text = oct)[[1]])))
invisible(Sys.setlocale("LC_CTYPE", "C"))
r <- "café"
print(Encoding(r)); print(r); cat(r, "\n"); print(nchar(r, "bytes"))
f <- function() "café"
print(Encoding(f())); print(f())
w <- eval(parse(text = lit)[[1]])
print(Encoding(w)); print(w)
print(Encoding(eval(parse(text = oct)[[1]])))
g <- eval(parse(text = paste0("function() ", lit)))
print(Encoding(g())); print(g())
invisible(Sys.setlocale("LC_CTYPE", "C.UTF-8"))
print(Encoding("café"))
print(r); print(w)
