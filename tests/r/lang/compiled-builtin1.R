# One-argument builtins and string equality in compiled code
probe <- function(x) list(length(x), is.null(x), is.atomic(x), is.character(x),
  is.logical(x), is.numeric(x), is.function(x), is.list(x), attributes(x), typeof(x))
vals <- list(1.5, NA_real_, NaN, 2L, NA_integer_, TRUE, NA, "a", NA_character_,
  NULL, c(a = 1), 1:3, factor("u"), list(1, 2), sum, probe, quote(x + y),
  as.name("s"), 3i, as.raw(1), structure(1, class = "foo"))
for (v in vals) print(probe(v))
nas <- function(x) is.na(x)
for (v in vals[c(1:10, 13, 21)]) print(nas(v))
nas(c(1, NA))
nas(list(1, NA))
r <- function(n) { s <- 1:n; c(length(s), is.na(s)[1], typeof(s), is.numeric(s)) }
r(1)
r(4)
rr <- function(n) { s <- 1:n; is.na(s) }
rr(1)
rr(3)
length.foo <- function(x) 99L
is.na.foo <- function(x) "dispatched"
is.numeric.foo <- function(x) "numeric?"
obj <- structure(1, class = "foo")
probe(obj)
nas(obj)
e <- new.env(); assign("a", 1, e); assign(".b", 2, e)
probe(e)[[1]]
seqlen <- function(n) { k <- 0L; for (i in 1:n) k <- k + length(i); k }
seqlen(50)
countna <- function(x) { k <- 0L; for (v in x) if (is.na(v)) k <- k + 1L; k }
countna(c(1, NA, 3, NA, NaN))
countna(c("a", NA, "b"))
eqs <- function(a, b) c(a == b, a != b)
eqs("x", "x")
eqs("x", "y")
eqs("x", NA_character_)
eqs(NA_character_, NA_character_)
eqs("é", "é")
eqs(c(a = "x"), "x")
eqs(factor("x"), "x")
tally <- function(xs, key) { k <- 0L; for (s in xs) if (s == key) k <- k + 1L; k }
tally(c("a", "b", "a", "c", "a"), "a")
local({
  length <- function(x) -1L
  f <- function(v) length(v)
  print(f(1:3))
})
local({
  is.null <- function(x) "shadowed"
  f <- function(v) is.null(v)
  print(f(NULL))
})
tf <- function(x) typeof(x)
tf(1:2)
tf(sum)
tf(quote(a))
vis <- function(x) invisible(length(x))
vis(1)
withVisible(vis(1))
withVisible((function(x) is.null(x))(1))
