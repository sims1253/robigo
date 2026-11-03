# Whole closures in native code (jit/closure.rs): values of any type,
# lists, `[[`, `c()`, builtins of one argument. Each must behave as in the VM.
hot <- function(f, ...) {
  a <- list(...)
  n <- length(a)
  if (n == 0) for (i in 1:600) f()
  else if (n == 1) { x <- a[[1]]; for (i in 1:600) f(x) }
  else if (n == 2) { x <- a[[1]]; y <- a[[2]]; for (i in 1:600) f(x, y) }
  else stop("hot")
}

# trees of lists
tree <- function(item, depth) {
  if (depth == 0L) return(c(item, NA, NA))
  list(item, tree(2L * item - 1L, depth - 1L), tree(2L * item, depth - 1L))
}
chk <- function(t) if (is.na(t[[2]][[1]])) t[[1]] else t[[1]] + chk(t[[2]]) - chk(t[[3]])
hot(tree, 1L, 3L)
t3 <- tree(1L, 3L)
hot(chk, t3)
print(chk(t3)); print(chk(tree(5L, 6L))); dput(tree(1L, 1L))
print(chk(c(4L, NA, NA)))
# a deep tree near the recursion limit stays correct
print(chk(tree(1L, 12L)))
# `[[` out of range, of NULL, of a name, negative, NA
g1 <- function(x, i) x[[i]]
hot(g1, list(1, "a", TRUE), 2)
print(g1(list(1, "a", TRUE), 3)); print(g1(list(1, "a", TRUE), 2.9))
r <- tryCatch(g1(list(1, 2), 3), error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(g1(list(1, 2), 0), error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(g1(list(1, 2), -1), error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(g1(list(1, 2), NA), error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(g1(NULL, 1), error = function(e) conditionMessage(e)); print(r)
print(g1(list(a = 1, b = 2), "b"))
print(g1(c(x = 1.5, y = 2.5), 2)); print(g1(letters, 3)); print(g1(1:5, 4L))
print(g1(list(list(1, 2), 3), 1))
# `[` of a list stays a list (the VM's call)
g2 <- function(x, i) x[i]
hot(g2, list(1, 2), 1)
print(g2(list(1, 2), 2)); print(g2(c(a = 1, b = 2), 1)); print(g2(1:3, 2))
# classed objects dispatch
`[[.box` <- function(x, i) { cat("method\n"); unclass(x)[[i]] * 10 }
bx <- structure(list(1, 2), class = "box")
hot(g1, list(1, 2), 1)
print(g1(bx, 2))
length.box <- function(x) 99L
ln <- function(x) length(x) + 0L
hot(ln, list(1, 2))
print(ln(bx)); print(ln(list(1, 2, 3))); print(ln(NULL)); print(ln("a"))
# builtins of one argument on values of any type
b1 <- function(x) c(is.null(x), is.na(x[[1]]), is.list(x), is.character(x), is.numeric(x))
hot(b1, list(1))
print(b1(list(NA))); print(b1(list("a"))); print(b1(NULL)); print(b1(c(1, 2)))
r <- tryCatch(b1(list()), error = function(e) conditionMessage(e)); print(r)
ty <- function(x) typeof(x[[1]])
hot(ty, list(1L))
print(ty(list("a"))); print(ty(list(NULL))); print(ty(list(sum))); print(ty(list(list())))
# c() of scalars: types, NA, coercion; anything else is the VM's
cc <- function(a, b) c(a, b, NA)
hot(cc, 1L, 2L)
print(cc(1L, 2L)); print(cc(1, 2L)); print(cc(TRUE, FALSE)); print(cc(TRUE, 2L)); print(cc(NA, NA))
print(cc(1.5, NA_integer_)); print(cc("a", 1)); print(cc(list(1), 2)); print(cc(c(1, 2), 3))
print(cc(factor("a"), 1)); print(cc(NULL, 1)); print(typeof(cc(TRUE, 1L)))
print(cc(c(a = 1), 2))
# list() keeps every value; takes its arguments only once
mk <- function(a, b) list(a, b, list(a))
hot(mk, 1, "x")
dput(mk(1, "x")); dput(mk(NULL, sum)); dput(mk(list(1), quote(y)))
x <- mk(1:3, 2); x[[1]][1] <- 99L; dput(x)
v <- 1:3; y <- mk(v, v); v[1] <- 0L; dput(y)
# arithmetic and comparisons on values of any type
ar <- function(x) x[[1]] * 2 + x[[2]]
hot(ar, list(1, 2))
print(ar(list(1L, 2L))); print(ar(list(1.5, TRUE))); print(typeof(ar(list(1L, 2L))))
r <- withCallingHandlers(ar(list(.Machine$integer.max, 1L)), warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
print(r)
r <- tryCatch(ar(list("a", 1)), error = function(e) conditionMessage(e)); print(r)
print(ar(list(c(1, 2), 1))); print(ar(list(NA, 1)))
rl <- function(x) if (x[[1]] > 0) "pos" else "non"
hot(rl, list(1))
print(rl(list(-1))); print(rl(list(0L)))
r <- tryCatch(rl(list(NA)), error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(rl(list(c(1, 2))), error = function(e) conditionMessage(e)); print(r)
# an assigned formal of any type, and locals
af <- function(x) { x <- x[[1]]; x <- x[[1]]; x }
hot(af, list(list(5)))
print(af(list(list(5)))); orig <- list(list(5)); af(orig); dput(orig)
lc <- function(x) { y <- x; z <- y[[2]]; list(z, y[[1]]) }
hot(lc, list(1, 2))
dput(lc(list("a", "b")))
# a list from an enclosing environment
G <- list(10, 20, 30)
gr <- function(i) G[[i]] + 1
hot(gr, 2)
print(gr(3)); G <- list("a"); r <- tryCatch(gr(1), error = function(e) conditionMessage(e)); print(r)
G <- list(1, 2); print(gr(2))
# visibility
vl <- function(x) { y <- list(x); y }
hot(vl, 1)
vl(1)
print(withVisible(vl(1))$visible)
iv <- function(x) invisible(list(x))
hot(iv, 1)
iv(1)
# `list` and `c` rebound
cl <- function(a) list(a, a)
hot(cl, 1)
list <- function(...) "mine"
print(cl(1))
rm(list)
print(length(cl(1)))
c <- function(...) "mine too"
print(cc(1, 2))
rm(c)
print(cc(1, 2))
# ten cells: the cached variant must match all their types
a1 <- 1; a2 <- 2; a3 <- 3; a4 <- 4; a5 <- 5; a6 <- 6; a7 <- 7; a8 <- 8; a9 <- 9; a10 <- 10
g10 <- function() a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9 + a10
f10 <- function() g10()
for (i in 1:600) f10()
a1 <- list(100)
h10 <- function() f10()
print(tryCatch(h10(), error = function(e) conditionMessage(e)))
a1 <- 1L
print(h10())
# a logical subscript selects, it is not a position
gl <- function(x, y) x[[1]][y > 0]
fl <- function(x, y) gl(x, y)
xl <- list(c(10, 20, 30))
for (i in 1:600) fl(xl, 1)
hl <- function() fl(xl, 1)
print(hl()); print(fl(xl, -1))
gl2 <- function(x, y) x[[y > 0]]
for (i in 1:600) gl2(list(5, 6), 1)
print(tryCatch(gl2(list(5, 6), 1), error = function(e) conditionMessage(e)))
# c() of nothing is NULL
z0 <- function() c()
f0 <- function(x) z0()
for (i in 1:600) f0(1)
print(f0(1)); print(is.null(f0(1)))
l0 <- function() list()
for (i in 1:600) l0()
print(l0())
