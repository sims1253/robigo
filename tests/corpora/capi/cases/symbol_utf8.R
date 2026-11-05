dyn.load("symbol_utf8.so")

# Symbol-font letters (a = alpha, b = beta, g = gamma, d = delta, W = Omega,
# \xa5 = infinity, \xb3 = greater-or-equal) become their Unicode characters
x <- c("a", "b", "g", "d", "abgd", "W", "\xa5", "\xb3", "plain 1+2")
utf8 <- .Call("capi_symbol_utf8", x, 1L)    # CE_UTF8
print(utf8)
print(Encoding(utf8))
print(lapply(utf8, utf8ToInt))
# to the native encoding (or Latin-1) Symbol text is left as it is
print(identical(.Call("capi_symbol_utf8", x[1:5], 0L), x[1:5]))   # CE_NATIVE
print(identical(.Call("capi_symbol_utf8", x[1:5], 2L), x[1:5]))   # CE_LATIN1
