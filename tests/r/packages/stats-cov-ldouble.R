# cov/cor/var/sd: long double sums in x87 loops (bit-identical)
library(stats)
h <- function(v) cat(sprintf("%a", v), "\n")
i <- 1:5000
x <- sin(i) * 10^((i %% 7) - 3); y <- cos(i * 1.3) + x / 3; z <- (i %% 17) / 3
h(cor(x, y)); h(cov(x, y)); h(var(x)); h(sd(y)); h(mean(x))
m <- cbind(x, y, z, x * y)
h(cor(m)); h(cov(m)); h(var(m, m[, 2:3]))
h(cor(m, method = "spearman")); h(cor(x[1:300], y[1:300], method = "kendall"))
x2 <- x; x2[c(10, 200)] <- NA
h(cor(x2, y)); h(cor(x2, y, use = "complete.obs")); h(cor(cbind(x2, y), use = "pairwise.complete.obs"))
h(var(c(1e10 + 1:5)))
h(cor(c(1, 1, 1), c(1, 2, 3)))
x3 <- c(TRUE, FALSE, NA) & c(TRUE, TRUE, TRUE); x3
(1:6 > 2) | c(TRUE, FALSE)
anyDuplicated(c(3, 1, 2, 1)); anyDuplicated(c("a", NA, "b", NA), fromLast = TRUE); anyDuplicated(c(1, NA, 1), incomparables = 1)
as.character(c(0L, 65535L, 65536L, -5L, NA))
