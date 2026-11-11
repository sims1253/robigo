# R: builtin.c:asVecSize reads a string size with asReal, which warns
# "NAs introduced by coercion" before the NA/NaN error.
w <- function(expr) withCallingHandlers(
  tryCatch(expr, error = function(e) conditionMessage(e)),
  warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
print(w(vector("list", "a")))
print(w(readLines(textConnection("a\nb"), n = "x")))
print(w(.Internal(colSums(1:4, "a", 2L, FALSE))))
print(w(sample(5, "b")))
x <- 1:3
print(w(length(x) <- "2")); print(x)
print(w(length(x) <- "a")); print(x)
