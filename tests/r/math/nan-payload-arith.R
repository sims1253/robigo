# Which of two NaN operands survives double + * - / (NA or NaN): GNU R's
# compiled loops keep the destination operand's payload, which depends on
# the shape (scalar/vector, recycling) and on an integer operand.
cls <- function(v) paste(ifelse(is.na(v) & !is.nan(v), "A", ifelse(is.nan(v), "N", "-")), collapse = "")
n <- 0/0; a <- NA_real_; i <- NA_integer_
show <- function(label, v) cat(sprintf("%-10s %s\n", label, cls(v)))
for (op in c("+", "*", "-", "/")) {
  f <- match.fun(op)
  cat("op", op, "\n")
  # variables (fresh result)
  x2 <- c(a, n); y2 <- c(n, a); x4 <- c(a, n, a, n)
  show("vv", f(x2, y2)); show("vs", f(x2, n)); show("sv", f(n, y2)); show("rc", f(x4, y2))
  show("ss", c(f(a, n), f(n, a)))
  # temporaries (computed in place)
  show("vv tmp", f(c(a, n), c(n, a))); show("vs tmp", f(c(a, n), n + 0)); show("sv tmp", f(a + 0, c(n, a)))
  show("rc tmp", f(c(a, n, a, n), c(n, a)))
  # an integer operand
  show("ii ss", c(f(i, n), f(n, i))); show("ii vv", c(f(c(i, i), c(n, n)), f(c(n, n), c(i, i))))
  show("ii sv", c(f(i, c(n, n)), f(n, c(i, i)))); show("ii vs", c(f(c(i, i), n), f(c(n, n), i)))
  show("ii rc", c(f(c(i, i, i, i), c(n, n)), f(c(n, n, n, n), c(i, i))))
  # logical NA counts as a double
  show("lgl", c(f(NA, n), f(n, NA), f(c(NA, NA), c(n, n)), f(NA, c(n, n)), f(c(NA, NA, NA, NA), c(n, n))))
  # long vectors (the unrolled loops) and a column recycled over a matrix
  v <- rep(c(a, n, 1), 7); w <- rep(c(n, a, a), 7)
  show("long", f(v, w)); show("long tmp", f(v + 0, w + 0)); show("long s", f(v, n))
  m <- matrix(c(a, n, a, n, n, a), 2); show("matcol", f(m, c(n, a)))
}
f <- function(a, b, c) a * b + c
print(f(c(NA, NaN), c(NaN, NA), 1))
