# substitute() leaves an unbound `...` as LCONS(R_DotsSymbol, R_NilValue):
# a LANGSXP cell inside the call, which serialize() writes (R: coerce.c
# substituteList). Bound dots expand to ordinary cells.
x <- substitute(f(a, ...), list(a = 1))
print(serialize(x, NULL, version = 2))
y <- unserialize(serialize(x, NULL))
print(identical(serialize(y, NULL), serialize(x, NULL)))
print(identical(x, quote(f(1, ...))))
print(serialize(quote(f(1, ...)), NULL, version = 2))
g <- function(...) substitute(h(..., z))
print(serialize(g(1, b = 2), NULL, version = 2))
print(serialize(substitute(k(...)), NULL, version = 2))
