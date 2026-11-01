# NA_integer_ from an inlined closure must become NA_real_ (not i32::MIN)
# on sapply/vapply's fast path, and stay NA_integer_ on the integer path.
f <- function(x) if (x == 2) NA_integer_ else x
print(sapply(1:4, f))
print(vapply(1:4, f, 0))
print(vapply(1:4, f, 1L))
g <- function(x, k) if (x == k) NA_integer_ else x + k
print(vapply(1:4, function(x) g(x, 2L), 0))
