# Recursive Fibonacci: closure call overhead.
fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
cat(fib(25), "\n")
