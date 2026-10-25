f <- function(x) {
  if (x < 0) stop("x must be non-negative")
  sqrt(x)
}
f(4)
f(-1)
cat("not reached\n")
