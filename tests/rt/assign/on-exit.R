# on.exit code runs when the function returns, in order
log <- 0
f <- function() {
  on.exit(log <<- log * 10 + 1)
  on.exit(log <<- log * 10 + 2, add = TRUE)
  on.exit(log <<- log * 10 + 3, add = TRUE, after = FALSE)
  log <<- 9
  5
}
f()
log
g <- function() {
  on.exit(log <<- 0)
  on.exit()
  7
}
g()
log
