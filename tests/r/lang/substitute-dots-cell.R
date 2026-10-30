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
# Cell types must survive beyond the old 64-argument mask and repeated use.
for (n in c(0L, 1L, 63L, 64L, 65L, 130L)) {
    q <- as.call(c(list(as.name('f')), rep(list(1), n), list(as.name('...'))))
    x <- substitute(EXPR, list(EXPR = q))
    x <- do.call('substitute', list(x, list()))
    raw <- serialize(x, NULL, version = 2)
    print(c(n, length(raw), sum(as.integer(raw))))
    print(identical(raw, serialize(unserialize(raw), NULL, version = 2)))
}
print(serialize(substitute(f(tag = ...), list()), NULL, version = 2))
f <- function(...) substitute(g(...))
print(f())
print(f(, a = 1))
# Read head attributes before registering the exceptional tail cells.
e <- structure(quote(f(...)), test = 1L)
x <- do.call('substitute', list(e, list()))
r <- serialize(x, NULL, version = 2)
print(identical(r, serialize(unserialize(r), NULL, version = 2)))
