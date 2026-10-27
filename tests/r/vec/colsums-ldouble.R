# colSums/rowSums/colMeans/rowMeans and cumprod with long double accumulators
h <- function(v) cat(sprintf("%a", v), "\n")
i <- 1:6000
m <- matrix(sqrt(i) * 10^((i %% 13) - 6) / 3, 60)
h(colSums(m)); h(colMeans(m)); h(rowSums(m)); h(rowMeans(m))
m2 <- m; m2[c(5, 77, 300, 301)] <- c(NA, NaN, NA, 1e308); m2[400:460] <- NaN
for (r in c(FALSE, TRUE)) { h(colSums(m2, na.rm = r)); h(colMeans(m2, na.rm = r)); h(rowSums(m2, na.rm = r)); h(rowMeans(m2, na.rm = r)) }
mi <- matrix(as.integer(i %% 1000 - 300), 60); mi[c(3, 100, 2000)] <- NA
for (r in c(FALSE, TRUE)) { h(colSums(mi, na.rm = r)); h(colMeans(mi, na.rm = r)); h(rowSums(mi, na.rm = r)); h(rowMeans(mi, na.rm = r)) }
ml <- matrix(i %% 3 == 0, 60); ml[7] <- NA
h(colSums(ml)); h(rowMeans(ml, na.rm = TRUE))
h(colMeans(matrix(numeric(0), 0, 3))); h(rowMeans(matrix(NaN, 2, 2), na.rm = TRUE))
x <- 1 + sin(1:5000) / 1000
h(cumprod(x)); h(cumprod(c(2, NA, 3))); h(cumprod(c(1e200, 1e200, 0)))
