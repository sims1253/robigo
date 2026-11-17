# substitute(expr, <list>): the list's names are installed as
# VectorToPairList does (installTrChar), so a latin1-marked name binds the
# symbol with the same text.
x <- list(42)
nm <- "caf\xe9"
Encoding(nm) <- "latin1"
names(x) <- nm
print(substitute(café + 1, x))
print(substitute(a + b, list(a = 1, b = quote(z))))
