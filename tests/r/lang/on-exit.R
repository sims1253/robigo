f <- function() {
  on.exit(cat("bye\n"))
  cat("hello\n")
  invisible(1)
}
f()
g <- function() {
  on.exit(cat("first\n"))
  on.exit(cat("second\n"), add = TRUE)
  cat("body\n")
}
g()
h <- function() {
  on.exit(cat("cleanup\n"))
  stop("boom")
}
r <- tryCatch(h(), error = function(e) conditionMessage(e))
r
