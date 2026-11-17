# Byte code's VECSUBSET and VECSUBSET2 (eval.c:DO_VECSUBSET, DO_FAST_VECELT,
# FAST_VECELT_OK, bcStackIndex): x[i] and x[[i]] with one scalar index,
# compiled (each in a function with a loop, which the JIT compiles), for
# every type the fast path reads and the cases that fall to the default.
loop <- function(f) { r <- NULL; for (k in 1:2) r <- f(); r }
s1 <- function(x, i) loop(function() x[[i]])
s0 <- function(x, i) loop(function() x[i])
show <- function(v) { print(v); print(typeof(v)) }
vals <- list(dbl = c(1.5, 2.5, NA), int = c(4L, NA, 6L), lgl = c(TRUE, NA, FALSE),
             cplx = c(1+2i, 3-1i), raw = as.raw(c(1, 255)), lst = list(1, "b", NULL, list(2)))
for (nm in names(vals)) {
    x <- vals[[nm]]
    cat("--", nm, "\n")
    show(s1(x, 2L))
    show(s1(x, 2))
    show(s0(x, 1L))
    show(s0(x, 2.9))
}
# an index in a boxed scalar, and one that is a real above an integer
i <- c(2L); show(s1(vals$dbl, i)); show(s1(vals$int, 2.0000001))
# names: `[[` drops them, `[` keeps them (the default)
nx <- c(a = 1, b = 2); show(s1(nx, 2L)); show(s0(nx, 2L))
nl <- list(a = 1, b = "z"); show(s1(nl, 2L)); show(s0(nl, 2L))
# dim only: `[` takes the fast path, the element without dim
m <- matrix(1:6, 2); show(s0(m, 5L)); show(s1(m, 6L))
# indices that fall to the default
show(s0(vals$dbl, NA_integer_)); show(s0(vals$dbl, 0L)); show(s0(vals$dbl, -1L))
show(s0(vals$int, TRUE)); show(s0(vals$lgl, 4L)); show(s0(vals$lst, 9L))
print(tryCatch(s1(vals$dbl, 4L), error = conditionMessage))
print(tryCatch(s1(vals$lst, 0L), error = conditionMessage))
print(tryCatch(s1(vals$int, NA_integer_), error = conditionMessage))
print(tryCatch(s1(vals$dbl, -1L), error = conditionMessage))
print(tryCatch(s1(vals$lst, 5), error = conditionMessage))
# a compact sequence (ALTREP) and a character vector: the default
show(s1(1:10, 3L)); show(s0(1:10, 4L)); show(s1(c("p", "q"), 2L)); show(s0(c("p", "q"), 1L))
# the element a `[[` of a list gives is the element: changing the copy
# leaves the list as it was
l <- list(c(1, 2), 3)
f <- function() { for (k in 1:2) { e <- l[[1L]]; e[1] <- 99 }; e }
print(f()); print(l[[1L]])
