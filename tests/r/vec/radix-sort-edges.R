# Radix and shell ordering across the sort paths: counting, packed and
# MSD radix, sorted/reversed input, NA placement, ties, strings in byte
# order (radix) versus collation (shell), grouping.
ck <- function(o) cat(length(o), sum(as.numeric(o) * seq_along(o)), o[seq_len(min(6L, length(o)))], "\n")
set.seed(42)

# integers: small range (counting), wide range (packed radix), small n
xi <- sample(1e6, 2000, replace = TRUE)
xi[c(5, 77, 1500)] <- NA
for (m in c("radix", "shell")) for (nl in c(TRUE, FALSE, NA)) for (dec in c(FALSE, TRUE)) {
  ck(order(xi, na.last = nl, decreasing = dec, method = m))
}
ck(order(c(.Machine$integer.max, -.Machine$integer.max, NA, 0L, 5L)[rep(1:5, 80)], method = "radix"))
ck(order(c(.Machine$integer.max, -.Machine$integer.max, NA, 0L, 5L)[rep(1:5, 80)], decreasing = TRUE, method = "radix"))
ck(order(sample(50, 300, replace = TRUE), method = "radix"))
ck(order(sample(5, 40, replace = TRUE), decreasing = TRUE, method = "radix"))
ck(sort(xi, method = "radix"))
ck(sort(xi, decreasing = TRUE, method = "radix"))

# doubles: wide keys (MSD), ties, -0, NaN vs NA, Inf, small n
xd <- c(sin(1:3000) * 1e5, rep(c(0, -0, NA, NaN, Inf, -Inf, 1/3), 20))
xd <- xd[sample(length(xd))]
for (m in c("radix", "shell")) for (nl in c(TRUE, FALSE, NA)) for (dec in c(FALSE, TRUE)) {
  ck(order(xd, na.last = nl, decreasing = dec, method = m))
}
ck(order(round(xd, 1), method = "radix"))
ck(order(c(2.5, NA, -1, NaN, 0, -0, 3, 2.5), method = "radix"))
ck(order(c(2.5, NA, -1, NaN, 0, -0, 3, 2.5), na.last = FALSE, decreasing = TRUE, method = "radix"))
ck(order(sin(1:40 * 7), method = "radix"))
ck(order(c(1e300, -1e300, 1e-300, -1e-300, 5e-324)[rep(1:5, 30)], method = "radix"))
ck(sort(xd, method = "radix"))

# grouping rounds doubles (dround = 2): near-equal values tie
g <- .Internal(radixsort(TRUE, FALSE, TRUE, TRUE, c(1, 1 + 2^-50, 1 + 2^-40, 1, 2)))
print(attributes(g)); ck(g)

# strings: radix is byte order, shell is collation
s <- c("b", "B", "a", "A", NA, "ab", "aB", "_x", "Z", "", "b", NA, "a")
for (m in c("radix", "shell")) for (nl in c(TRUE, FALSE, NA)) for (dec in c(FALSE, TRUE)) {
  print(order(s, na.last = nl, decreasing = dec, method = m))
}
print(sort(s, method = "radix")); print(sort(s, method = "shell"))
sl <- sample(c(letters, LETTERS, NA, "aa", "Ab"), 1000, replace = TRUE)
ck(order(sl, method = "radix")); ck(order(sl, method = "shell"))
ck(order(sl, decreasing = TRUE, na.last = FALSE, method = "radix"))

# multiple keys, NA in a later key, mixed decreasing
k1 <- sample(4, 600, replace = TRUE); k2 <- sample(c(1.5, 2, NA, 3), 600, replace = TRUE)
k3 <- sample(c("x", "y", NA), 600, replace = TRUE)
ck(order(k1, k2, k3, method = "radix"))
ck(order(k1, k2, k3, method = "shell"))
ck(order(k1, k2, k3, na.last = NA, method = "radix"))
ck(order(k1, k2, k3, decreasing = c(TRUE, FALSE, TRUE), method = "radix"))
ck(order(k1, k2, k3, decreasing = TRUE, na.last = FALSE, method = "shell"))
ck(order(k3, k1, method = "radix")); ck(order(k3, k1, method = "shell"))

# grouping(): first-appearance string groups, ends and maxgrpn
gr <- grouping(c("b", "a", "b", NA, "c", "a", NA))
print(as.vector(gr)); print(attr(gr, "ends")); print(attr(gr, "maxgrpn"))
gr <- grouping(k1, k3)
ck(gr); print(attr(gr, "ends")); print(attr(gr, "maxgrpn"))
gr <- grouping(c("c", "b", "a")); print(as.vector(gr)); print(attr(gr, "ends"))

# na.last = NA with groups returned: NA rows become 0
r <- function(...) .Internal(radixsort(NA, rep(FALSE, ...length()), TRUE, TRUE, ...))
print(as.vector(r(c(3L, NA, 1L, NA, 2L))))
print(as.vector(r(c(5L, NA))))
print(as.vector(r(c(NA, 5L))))
print(as.vector(r(c(2, 1, 2, 1), c(7, NA, 3, 1))))
print(as.vector(r(c("b", NA))))
print(as.vector(r(c(1L, 1L, 2L, 2L), c(9L, NA, NA, 4L))))
print(as.vector(.Internal(radixsort(NA, FALSE, FALSE, TRUE, c(NA_real_, NA_real_)))))
print(as.vector(.Internal(radixsort(NA, c(FALSE, TRUE), FALSE, TRUE, c(1L, 1L, 2L), c(NA, 3, 4)))))

# the sorted and reversed shortcuts with ties and NA
print(order(c(1L, 1L, 2L, NA, NA), method = "radix"))
print(order(c(NA, 3, 2, 1), method = "radix"))
print(order(c(3, 2, 1, NA), na.last = NA, method = "radix"))
print(order(c(3, 3, 2, 1), method = "radix"))
print(order(c(1L, 2L, 3L), decreasing = TRUE, method = "radix"))
