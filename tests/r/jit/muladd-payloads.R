# Preserve raw NA/NaN payloads when combining arithmetic operations.
hex2d <- function(h) vapply(h, function(s)
  readBin(as.raw(strtoi(substring(s, seq(15, 1, -2), seq(16, 2, -2)), 16L)),
          "double"), 0, USE.NAMES = FALSE)
v <- hex2d(c("7ff00000000007a2", "7ff8000000000123", "fff4000000000456", "3ff0000000000000"))
f <- function(a, b, c) a * b + c
g <- function(a, b, c) c + a * b
h <- function(a, b, c) a * b - c
k <- function(a, b, c) c - a * b
# Calling primitives through variables supplies an independent unfused
# reference. Compare bytes because identical() considers NaN payloads equal.
mul <- .Primitive("*")
add <- .Primitive("+")
sub <- .Primitive("-")
refs <- list(function(a, b, c) add(mul(a, b), c),
             function(a, b, c) add(c, mul(a, b)),
             function(a, b, c) sub(mul(a, b), c),
             function(a, b, c) sub(c, mul(a, b)))
funs <- list(f, g, h, k)
checked <- 0L
for (width in c(2L, 8L, 9L)) for (i in seq_along(v)) for (j in seq_along(v)) for (l in seq_along(v)) {
  a <- rep(v[i], width); b <- rep(v[j], width); z <- rep(v[l], width)
  for (op in seq_along(funs)) {
    fun <- funs[[op]]; ref <- refs[[op]]
    for (shape in 0:7) {
      x <- if (bitwAnd(shape, 1L) != 0L) a[1] else a
      y <- if (bitwAnd(shape, 2L) != 0L) b[1] else b
      w <- if (bitwAnd(shape, 4L) != 0L) z[1] else z
      stopifnot(identical(writeBin(fun(x, y, w), raw()),
                          writeBin(ref(x, y, w), raw())))
      checked <- checked + 1L
    }
  }
}
print(checked)
