s <- 0
fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)

g <- function() { for (i in 1:20) { s <- s + fib(18) }; NULL }
g()
