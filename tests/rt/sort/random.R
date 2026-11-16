# Large sorts of generated data, the same in every R: a multiplicative
# hash of the index (exact in doubles) picks integers with ties and NA,
# doubles with NA, NaN, -0, 0 and infinities, and strings of mixed pieces.
# Each result is summarized: a weighted checksum and a sample of positions.
n <- 20000
h <- ((1:n) * 2654435761) %% 4294967296
k <- function(o) c(length(o), sum(o * ((seq_along(o) - 1) %% 1009 + 1)), o[seq(1, length(o), by = 997)])
# integers: about 2,000 values, so ties, and NA
xi <- as.integer(h %% 2001) - 1000L
xi[h %% 97 == 0] <- NA
for (m in c("radix", "shell")) {
  print(k(order(xi, method = m)))
  print(k(order(xi, method = m, decreasing = TRUE, na.last = FALSE)))
  print(k(order(xi, method = m, na.last = NA)))
}
print(k(sort(xi)))
print(k(sort(xi, method = "shell", decreasing = TRUE)))
print(k(sort(xi, method = "quick")))
print(k(rank(xi)))
print(k(rank(xi, ties.method = "min", na.last = "keep")))
print(k(rank(xi, ties.method = "first")))
# doubles: magnitudes over seven decades, and the special values
xd <- (h / 4294967296 - 0.5) * 10^(h %% 7 - 3)
sp <- h %% 101
xd[sp == 0] <- NA
xd[sp == 1] <- NaN
xd[sp == 2] <- -0
xd[sp == 3] <- 0
xd[sp == 4] <- Inf
xd[sp == 5] <- -Inf
xd[sp == 6] <- (xd[sp == 6] * 4) %/% 1 / 4
for (m in c("radix", "shell")) {
  print(k(order(xd, method = m)))
  print(k(order(xd, method = m, decreasing = TRUE, na.last = FALSE)))
  print(k(order(xd, method = m, na.last = NA)))
}
for (m in c("radix", "shell", "quick")) {
  s <- sort(xd, method = m)
  print(c(length(s), which(1/s == -Inf)[1:5], sum(s[is.finite(s)])))
}
s <- sort(xd, decreasing = TRUE, method = "shell")
print(c(length(s), which(1/s == -Inf)[1:5]))
print(k(rank(xd)))
print(k(rank(xd, ties.method = "max", na.last = FALSE)))
# strings: two of these pieces, collated (shell) and by bytes (radix)
p <- c("a", "A", "b", "B", "\u00e9", "e\u0301", "e", "E", "ab", "a b", "a-b", "10", "9", "", "_", "\u00df",
       "ss", "\u0436", "\u0416", "\u4e00", "\u7e37", "\uac00", "\u3042", "\u30a2", "z", "Z", "\u00f8", "o")
xs <- paste0(p[1 + h %% length(p)], p[1 + (h %/% 97) %% length(p)], p[1 + (h %/% 9973) %% length(p)])
xs[h %% 89 == 0] <- NA
xs <- xs[1:5000]
for (m in c("shell", "radix")) {
  print(k(order(xs, method = m)))
  print(k(order(xs, method = m, decreasing = TRUE, na.last = FALSE)))
}
print(k(rank(xs)))
print(k(rank(xs, ties.method = "first", na.last = "keep")))
print(k(match(sort(xs), xs)))
print(k(match(sort(xs, method = "radix", decreasing = TRUE), xs)))
# several keys
print(k(order(xi, xd, method = "radix")))
print(k(order(xi, xd, method = "shell")))
print(k(order(xi[1:5000], xs, xd[1:5000], method = "shell", decreasing = TRUE)))
print(k(order(xi[1:5000] %% 7L, xs, method = "radix", decreasing = c(TRUE, FALSE))))
