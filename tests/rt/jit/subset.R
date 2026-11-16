# cmp.R's `[`, `[[` and `$` handlers in compiled code (GNU R 4.6.2): the
# object is evaluated and dispatched on first (tryDispatch: the method's
# arguments are the call's, promised, and its visibility is kept, where
# the interpreted special makes the value visible); then the indices run
# as compiled code (a symbol bound to a missing argument is an empty
# index, an inlined wrapper makes no context) and the default method's
# value is visible. The JIT compiles a global closure without a loop at
# its second call, so each runs three times.
x <- c(a = 1, b = 2, c = 3)
l <- list(p = 1, q = "z", r = list(s = 5))
f1 <- function(v, i) v[i]
f2 <- function(v, i) v[[i]]
f4 <- function(v) v[]
f7 <- function(v, n) v[[n, exact = FALSE]]
f8 <- function(v) v$q
f9 <- function(v) v$"p"
f10 <- function(v) v$r$s
f11 <- function(i) x[i]
f12 <- function(i) f11(i)
for (k in 1:3) {
  print(f1(x, 2)); print(f1(x, "c")); print(f1(l, 1:2))
  print(f2(l, "q")); print(f2(x, 3L)); print(f4(x))
  print(f7(list(abc = 7), "ab")); print(f8(l)); print(f9(l)); print(f10(l))
  print(f11()); print(f12())
}
v1 <- function(x) x[invisible(1)]
v1(1:3); v1(1:3); v1(1:3)
"[.foo" <- function(x, i) { print(substitute(i)); invisible("m") }
"[[.foo" <- function(x, i) paste("method [[", i)
"$.foo" <- function(x, name) paste("method $", name)
o <- structure(list(a = 1), class = "foo")
g1 <- function(x, j) x[j + 1]
g2 <- function(x) x[["a"]]
g3 <- function(x) x$a
g1(o, 1); g1(o, 1); g1(o, 1)
for (k in 1:3) print(c(g2(o), g3(o)))
e <- new.env(); e$z <- 9
g4 <- function(e) e$z
for (k in 1:3) print(g4(e))
for (k in 1:3) print(tryCatch(f2(x, 9), error = function(e) conditionMessage(e)))
for (k in 1:3) print(tryCatch(f2(l, "nope"), error = function(e) conditionMessage(e)))
for (k in 1:3) print(tryCatch(f8(1), error = function(e) conditionMessage(e)))
h <- function(v) v[is.vector(1, v)]
print(h("numeric")); print(h("numeric"))
h(1)
