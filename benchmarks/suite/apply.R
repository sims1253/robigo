# Functional idioms: sapply/vapply/lapply/Map/Reduce with closures.
n <- 2e5
v <- vapply(1:n, function(i) i %% 7 + 0.5, numeric(1))
l <- lapply(1:(n / 10), function(i) c(i, i^2))
s <- sum(v) + sum(sapply(l, function(p) p[2] - p[1]))
s <- s + Reduce(`+`, Map(function(a, b) a * b, 1:1000, 1000:1))
s <- s + sum(unlist(Filter(function(z) z %% 3 == 0, as.list(1:30000))))
cat(sprintf("%.1f", s), "\n")
