# The native pipe's call (gram.y xxpipe): lcons(fun, lcons(lhs, args)), so
# the cell that holds the first argument is a LANGSXP. serialize() writes
# each cell's type (6 for LANGSXP, 2 for LISTSXP).
hex <- function(e) paste(serialize(e, NULL, xdr = TRUE)[-(1:14)], collapse = " ")
hex(quote(x |> f(y)))
hex(quote(f(x, y)))
hex(quote(x |> f()))
hex(quote(x |> f(y) |> g(z = 1)))
identical(quote(x |> f(y)), quote(f(x, y)))
identical(quote(x |> f(y)), quote(x |> f(y)))
# a named placeholder and an extraction chain keep the RHS's own cells
hex(quote(x |> f(y = _)))
hex(quote(x |> _$a))
identical(quote(x |> f(y = _)), quote(f(y = x)))
# the call behaves as any other
f <- function(a, b = 2) a + b
1 |> f()
1 |> f(10)
e <- quote(1 |> f(5))
eval(e)
deparse(e)
e[[2]]
length(e)
as.list(e)
g <- function(a, b) match.call()
1 |> g(2)
h <- function(v) {
  for (i in 1:2) NULL
  v |> f(1)
}
h(3)
h(4)
e[[2]] <- 7
eval(e)
hex(e)
# a piped call in a body, seen and called in the usual ways
k <- function(v) v |> f(b = 1)
body(k)
deparse(k)
k(2)
s <- function(e) substitute(e)
s(1 |> f())
hex(s(1 |> f()))
sc <- function(a, b = 0) sys.call()
2 |> sc(3)
do.call("f", list(1, 2))
r <- unserialize(serialize(quote(x |> f(y)), NULL))
identical(hex(r), hex(quote(x |> f(y))))
e2 <- quote(1 |> f(5))
e2[-1]
hex(e2[-1])
as.call(as.list(e2))
hex(as.call(as.list(e2)))
