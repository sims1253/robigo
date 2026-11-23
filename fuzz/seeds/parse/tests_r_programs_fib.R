fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
sapply(0:15, fib)
fib_iter <- function(n) {
  a <- 0
  b <- 1
  for (i in seq_len(n)) {
    tmp <- a + b
    a <- b
    b <- tmp
  }
  a
}
fib_iter(50)
memo <- new.env()
fib_memo <- function(n) {
  key <- as.character(n)
  if (!is.null(memo[[key]])) return(memo[[key]])
  r <- if (n < 2) n else fib_memo(n - 1) + fib_memo(n - 2)
  memo[[key]] <- r
  r
}
fib_memo(80)
