# The console reads up to each `;` or newline: assignments are invisible,
# `(x <- v)` visible, `if` without `else` invisible when false; closures,
# recursion and `.Last.value`.
x <- 5; x
(y <- x * 2)
y = 3; y
if (x > 10) "big"
if (x > 1) "small"
if (FALSE) 1 else 2

# comments and blank lines print nothing
f <- function(n) if (n < 2) n else f(n - 1) + f(n - 2)
f(15)
g <- function(a, b = 2) {
  z <- a * b
  z + 1
}
g(3)
g(b = 10, 1)
h <- function() invisible_value <- 4
h()
.Last.value
k <- function(n) { s <- 0; i <- 0; while (i < n) { i <- i + 1; s <- s + i }; s }
k(100)
-x; !TRUE; (1 + 2) * 3; 7 %/% 2; -7 %% 3; 2^10; 5L/2L
"a" == "a"; 1L == 1.0; TRUE && NA; FALSE || NA
