# Base type rules: relop.c bitwise operations, list.c:do_allnames,
# builtin.c:xlengthgets / do_lengthgets, and getAttrib's compact row.names.
op <- function(x) print(x, digits = 15)
tried <- function(e) print(tryCatch(eval(e), error = function(x) conditionMessage(x)))

# bitwAnd/Or/Xor/Not coerce only REAL operands to integer and then require
# both operands to have the same type; shifts also coerce their count.
tried(quote(bitwAnd(TRUE, FALSE)))
tried(quote(bitwAnd(NULL, NULL)))
tried(quote(bitwAnd(c(1, 2, 3, 4), c(TRUE))))
tried(quote(bitwOr(1L, TRUE)))
tried(quote(bitwXor(as.raw(1), as.raw(2))))
tried(quote(bitwNot(TRUE)))
tried(quote(bitwNot(NULL)))
tried(quote(bitwShiftL(TRUE, c(TRUE, FALSE))))
tried(quote(bitwShiftL(NA, NA)))
tried(quote(bitwShiftL(1L, NULL)))
tried(quote(bitwShiftL(c(8, 4, 2), NULL)))
tried(quote(bitwShiftL(c(1, 2, 3, 4), c("a"))))
tried(quote(bitwShiftL(c(3, 2, 4), c(3 + 3i))))
tried(quote(bitwShiftR(c(1, 2, 3, 4), c("Hello"))))
tried(quote(bitwShiftR(1L, 1:2)))

# A shift count goes through coerceVector: pairlists and one-element lists
# coerce, longer list elements are an error.
op(bitwShiftL(1L, pairlist(1L)))
op(bitwShiftL(1L, list(1L)))
op(bitwShiftL(1L, list(1L, 2L)))
tried(quote(bitwShiftL(1L, list(1:2))))
op(bitwShiftL(1L, list("a")))
# A symbol coerces to NULL with its own warning, then fails the type check.
tried(quote(bitwShiftL(1L, quote(a))))
tried(quote(bitwShiftL(1L, as.name("x"))))
# List elements warn one by one.
op(bitwShiftL(1L, list("a", "b")))
op(bitwShiftL(1L, list(1L, "a", 2L)))
op(bitwShiftL(1L, pairlist("a", "b")))
# Only atomic elements with at least one value convert; a nested list or
# expression element is NA silently, and NULL is not vectorizable.
op(bitwShiftL(1L, list(list(1L))))
op(bitwShiftL(1L, list(list("a"))))
op(bitwShiftL(1L, list(expression(1L))))
op(bitwShiftL(1L, list(integer(0))))
tried(quote(bitwShiftL(1L, list(NULL))))
tried(quote(bitwShiftL(1L, pairlist(pairlist(1L)))))

# The coercions warn: characters give NA, complex drops the imaginary part,
# and each operand warns separately.
bitwShiftL(c(1, 2, 3, 4), c("a"))
bitwShiftL(c(3, 2, 4), c(3 + 3i))
bitwShiftL(c(1L, 2L), c(1.5, -1))
op(bitwAnd(Inf, Inf))
op(bitwShiftL(c(Inf, 1), c(1, Inf)))

# Values, including the 0..31 shift window and NA propagation.
op(bitwAnd(c(1L, 3L, NA_integer_), c(7L, 5L, 1L)))
op(bitwOr(-1L, 0L))
op(bitwXor(-1L, 0L))
op(bitwNot(2.5))
op(bitwNot(c(0L, 1L, -1L, NA_integer_)))
op(bitwShiftL(1L, -2:33))
op(bitwShiftR(-1L, -2:33))
op(bitwShiftL(1.5, 2))
op(bitwShiftR(256, 1:9))
op(bitwShiftL(c(1, 2), c(1L, 2L, 3L)))

# all.names/all.vars: max.names = -1 is unlimited, other negatives and NA
# mean no names; unique = NA counts as TRUE; functions = FALSE drops the
# call head.
op(all.names(expression(sin(x + y + x)), unique = NA))
tried(quote(all.names(as.symbol("a"), max.names = -2)))
tried(quote(all.names(quote(f(x)), max.names = NA_integer_)))
tried(quote(all.names(quote(f(x, y)), max.names = -1)))
op(all.names(quote(f(x, y)), functions = FALSE))
op(all.names(quote(f(g(x))), functions = FALSE, max.names = 1))
op(all.names(quote(f(g(x), g(y))), functions = FALSE, unique = TRUE, max.names = 2))
op(all.vars(quote(f(x, y) + g(z)), functions = TRUE))
op(all.names(quote(1)))
op(all.names(NULL))

# length<-: unchanged length keeps the object (attributes included), NULL
# stays NULL with a warning, and the size argument is asVecSize's.
a <- structure(c(3, 4), names = c("a", "b"), package = "methods")
length(a) <- 2
op(a)
op(attributes(a))
b <- 1:3
length(b) <- 5
op(b)
d <- list(1, "a")
length(d) <- 3
op(d)
e <- NULL
length(e) <- 2
op(e)
tried(quote({ x <- quote(a); length(x) <- 2 }))
tried(quote({ x <- 1:2; length(x) <- c(1, 2) }))
tried(quote(do.call("length<-", list(1:3, value = TRUE))))
tried(quote({ x <- 1:2; length(x) <- NA_integer_ }))
tried(quote({ x <- 1:2; length(x) <- NA_real_ }))
tried(quote({ x <- 1:2; length(x) <- Inf }))
tried(quote({ x <- 1:2; length(x) <- "3" }))
tried(quote({ x <- 1:2; length(x) <- -1 }))
# asVecSize errors come before xlengthgets' non-vector check.
tried(quote({ x <- quote(a); length(x) <- NA_real_ }))
tried(quote({ x <- quote(a); length(x) <- -1 }))
tried(quote({ x <- NULL; length(x) <- "1e100" }))
tried(quote({ x <- 1:2; length(x) <- "1e100" }))

# A compact c(NA, n) row.names prints expanded, through getAttrib.
df <- data.frame(a = 1:3)
u <- unclass(df)
print(u)
op(attributes(u)$row.names)
op(attr(u, "row.names"))
l <- structure(list(), row.names = c(NA, -4L), terms = quote(~0))
print(l)
op(attributes(l))
