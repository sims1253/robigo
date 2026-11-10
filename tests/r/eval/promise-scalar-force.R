# Argument promises whose code computes a scalar (`f(i * 0.5)`), forced by
# compiled code: the value is the same object for every reader, and lazy
# evaluation keeps its order.
g <- function(f) { r <- NULL; for (i in 1:3) r <- c(r, f(i * 0.5)); r }

# read unboxed, then as an object
f1 <- function(x) { y <- x + 1; list(y, x, typeof(x), identical(x, i * 0.5)) }
i <- 4
print(f1(i * 0.5))
print(g(function(x) { z <- x * 2; length(x) + z }))

# attributes and growth after an unboxed read: the promise's value is not
# changed by the copy
f2 <- function(x) { a <- x + 0; x[3] <- 7; names(x) <- c("a", "b", "c"); list(a, x) }
print(f2(i * 0.5))
f3 <- function(x) { a <- x * 1; attr(x, "u") <- "v"; x }
print(f3(i * 0.25))

# substitute and missing still see the promise
f4 <- function(x) { v <- x - 1; list(v, substitute(x), missing(x)) }
print(f4(i * 0.5))

# lazy: the caller's variable changes before the promise is forced
f5 <- function(x) { i <<- 100; x + 0 }
i <- 4
print(f5(i * 0.5))
i <- 4

# integer and logical results
f6 <- function(x) { y <- x + 1L; list(y, x, typeof(x)) }
print(f6(i %/% 3L))
k <- 5L
print(f6(k * 2L))
f7 <- function(x) { if (x) "yes" else "no" }
print(c(f7(i > 3), f7(i < 3), f7(i == 4)))
f8 <- function(x) { b <- !x; list(b, x) }
print(f8(i > 3))

# NA and NaN results
print(f6(NA_integer_ + k))
f9 <- function(x) { y <- x * 2; c(y, x, is.na(x)) }
print(f9(i * NA_real_))
print(f9(i * NaN))
print(f9(i / 0))

# the forced value read twice by objects is the same value
f10 <- function(x) { a <- x + 0; e <- environment(); identical(get("x", e), x) }
print(f10(i * 0.5))

# sys.function / match.call around such an argument
f11 <- function(x) { v <- x + 1; list(v, match.call()) }
print(f11(i * 0.5))

# a promise forced inside a loop, its frame kept by a closure
f12 <- function(x) { s <- 0; for (j in 1:3) s <- s + x; function() x }
h <- f12(i * 0.5)
print(h())
print(h() + 1)

# an error while forcing: the promise is forced again
f13 <- function(x) { r <- tryCatch(x + 1, error = function(e) "err"); r2 <- tryCatch(x + 1, error = function(e) "err2"); c(r, r2) }
print(f13(stop("boom")))
cnt <- 0
bump <- function() { cnt <<- cnt + 1; if (cnt < 2) stop("first") else cnt * 1.5 }
f14 <- function(x) { r <- tryCatch(x + 1, error = function(e) -1); r2 <- x + 1; c(r, r2) }
print(f14(bump()))

# an interrupted promise reached through another promise's symbol chase
delayedAssign("x", y)
invisible(tryCatch(x, error = function(e) NULL))
y <- 1
f15 <- function(z = x) z + 0
print(f15())
delayedAssign("x2", y2)
invisible(tryCatch(x2, error = function(e) NULL))
y2 <- 2
f16 <- function(z = x2) z + 0
options(warn = 2)
print(tryCatch(f16(), error = function(e) conditionMessage(e)))
options(warn = 0)
