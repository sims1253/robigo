# order() and sort() on character keys: collation order (ICU root in
# C.UTF-8; ASCII only, robigo does not have ICU's order for other
# characters), NA placement, ties kept in index order, several keys.
x <- c("b", "B", "a", "A", "ab", "aB", "Ab", "a b", "a-b", "a_b", "a.b",
       "10", "9", "1a", "", " ", "z", "Z", "_", "-", "(", "~", "$", "a",
       NA, "e", "f", "E", "u", "ab", "b")
o <- order(x)
print(o)
print(x[o])
print(order(x, decreasing = TRUE))
print(order(x, na.last = FALSE))
print(order(x, na.last = NA))
print(sort(x))
print(sort(x, decreasing = TRUE))
print(rank(x))
# several keys, ties broken by the next key and then by index
g <- c("b", "a", "B", "a", NA, "b", "A", "a", NA, "B")
v <- c(3, 1, 2, 1, 5, NaN, 2, NA, 1, 2)
print(order(g, v))
print(order(g, -v))
print(order(g, v, decreasing = TRUE))
print(order(g, v, na.last = FALSE))
print(order(v, g))
print(order(g, 10:1))
print(order(g, as.integer(v), rep(c(TRUE, FALSE), 5)))
print(order(g, complex(real = v, imaginary = 1)))
# a larger key with few distinct values (data-frame style)
set.seed(1)
k <- sample(c(letters[1:6], LETTERS[1:3], NA), 200, TRUE)
w <- sample(0:5, 200, TRUE)
print(order(k, -w))
print(order(k, decreasing = TRUE))
print(identical(order(k, w), order(k, w, method = "radix")))
# the collation follows Sys.setlocale
invisible(Sys.setlocale("LC_COLLATE", "C"))
print(sort(c("b", "B", "a", "A", "_", "1")))
print(order(c("b", "B", "a", "A"), 1:4))
invisible(Sys.setlocale("LC_COLLATE", "C.UTF-8"))
print(sort(c("b", "B", "a", "A", "_", "1")))
print(order(c("b", "B", "a", "A"), 1:4))
# errors: unsupported key types are still reported
print(tryCatch(order(c("a", "a"), list(1, 2)), error = conditionMessage))
print(order("a", list(1)))
