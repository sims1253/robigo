# order/sort/rank on integer keys: counting sort for small ranges, fewer radix passes
i <- 1:3000
xi <- as.integer((i * 31) %% 1000) - 500L; xi[c(5, 99)] <- NA
x <- (i * 7919) %% 10007 / 10007 - 0.5; x[c(7, 300)] <- c(NA, NaN)
s <- sprintf("k%03d", (i * 13) %% 400)
h <- function(v) cat(sum(v * seq_along(v)), v[1:5], v[(length(v) - 4):length(v)], "\n")
h(order(xi)); h(order(xi, decreasing = TRUE)); h(order(xi, na.last = FALSE)); h(order(xi, na.last = NA))
h(order(x)); h(order(-x, xi)); h(order(xi, x, decreasing = TRUE)); h(order(xi, s))
h(sort(xi)); h(sort(x, decreasing = TRUE)); h(sort.int(xi, index.return = TRUE)$ix)
h(order(c(2147483647L, -2147483647L, 0L, NA, 5L)))
h(order(rep(c(3L, 1L, 2L), 200)))
h(rank(xi, ties.method = "first")); h(order(as.integer(i %% 7), as.integer(i %% 3)))
