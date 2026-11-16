# Calls of pure closures from compiled code, which robigo runs inline:
# their values across types, branches, NA, strings, defaults and
# recursion. Each case runs three times in a compiled top-level loop: the
# first calls compile the closures, the last reaches the inline hook.
fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
run <- function() c(fib(0), fib(1), fib(10), fib(15L))
for (k in 1:3) r <- run()
print(r)
tag <- function(x, label) if (label == "bc") x * 2 else x
run <- function() {
  s <- 0
  for (i in 1:10) s <- s + tag(i * 0.25, "bc") + tag(i, "x")
  c(s, tag(label = "bc", x = 3), tag(2L, "bc"))
}
for (k in 1:3) r <- run()
print(r)
run <- function() tag(4, label = NA_character_ == "a")
for (k in 1:3) r <- tryCatch(run(), error = function(e) conditionMessage(e))
print(r)
ops <- function(a, b) c(a + b, a - b, a * b, a / b, a^b, a %% b, a %/% b, -a)
cmp <- function(a, b) c(a < b, a <= b, a == b, a != b, a >= b, a > b, !(a < b))
lg <- function(a, b) c(a && b, a || b, a & b, a | b)
run <- function() list(
  ops(7, 2), ops(7L, 2L), ops(-7L, 2L), ops(1, 0), ops(NA, 2),
  cmp(1, 2), cmp(2L, 2), cmp(NA_integer_, 1L), cmp(TRUE, FALSE),
  lg(TRUE, FALSE), lg(NA, TRUE), lg(NA, FALSE), lg(FALSE, NA),
  c(identical(ops(0.1, 0.2)[1], 0.1 + 0.2), is.nan(ops(0, 0)[4]), ops(2, 1023)[5])
)
for (k in 1:3) r <- run()
print(r)
str_eq <- function(x, y) x == y
run <- function() c(str_eq("a", "a"), str_eq("a", "b"), str_eq(NA_character_, "a"),
                    str_eq("café", "café"))
for (k in 1:3) r <- run()
print(r)
pick <- function(x, y, z) if (x) y else z
run <- function() c(pick(TRUE, 1, 2), pick(FALSE, 1, 2), pick(1, 3, 4), pick(0L, 3, 4), pick(2.5, 5, 6))
for (k in 1:3) r <- run()
print(r)
mutual_even <- function(n) if (n == 0) TRUE else mutual_odd(n - 1)
mutual_odd <- function(n) if (n == 0) FALSE else mutual_even(n - 1)
run <- function() c(mutual_even(10), mutual_odd(7), mutual_even(25))
for (k in 1:3) r <- run()
print(r)
kk <- function() 42
id <- function(x) x
run <- function() c(kk(), id(kk()), id(id(id(3))), id(NULL), length(id(1:3)), id(c(a = 1))[["a"]])
for (k in 1:3) r <- run()
print(r)
# defaults: constants (forced or not), a computed one (a real call), an
# empty argument, NULL
sb <- function(x, scale = 1, label = "a") if (label == "zz") x else x * scale
nd <- function(x, k = x * 2) x + k
run <- function() {
  s <- 0
  for (i in 1:5) s <- s + sb(i) + sb(i, 3) + sb(label = "zz", x = i) + nd(i) + nd(i, 1)
  list(s, sb(2, label = "b"), sb(1, NULL), sb(1, , "zz"))
}
for (k in 1:3) r <- run()
print(r)
run <- function() sb(4, label = NA_character_)
for (k in 1:3) r <- tryCatch(run(), error = function(e) conditionMessage(e))
print(r)
