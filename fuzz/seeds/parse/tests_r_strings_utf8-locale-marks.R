# Gap: in a UTF-8 locale, deparse() and format() build their results with
# mkChar, so non-ASCII results are native ("unknown"), not UTF-8-marked.
invisible(Sys.setlocale("LC_CTYPE", "C.UTF-8"))
u <- intToUtf8(233)
Encoding(deparse(u))
Encoding(deparse(quote(f(u))))
Encoding(format(u))
Encoding(paste(u))
