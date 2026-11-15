# Compiled code's generic call of a builtin (GNU R 4.6.2: bcEval's CALL
# after MAKEPROM, DOMISSING, DODOTS): a builtin reached by another name,
# with `...` or an empty argument evaluates its argument calls by their
# compiled code (an inlined wrapper makes no context), and an empty
# argument is an error only after the others are evaluated.
s <- sum
f <- function(m) { for (i in 1:2) NULL; s(1, is.vector(1, m)) }
print(f("any"))
g <- function(...) { for (i in 1:2) NULL; sum(2, ...) }
print(g(3))
h <- function() { for (i in 1:2) NULL; c(1, , cat("evaluated\n")) }
print(tryCatch(h(), error = function(e) conditionMessage(e)))
k <- function(m, ...) { for (i in 1:2) NULL; sum(is.vector(1, m), ...) }
print(k("any", 1))
f(1)
