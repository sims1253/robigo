# %% and %/% on doubles: long double remainder, c_eps = LDBL_EPSILON
h <- function(v) cat(sprintf("%a", v), sep = "\n")
i <- 1:300
x <- sqrt(i) * 10^((i %% 9) - 4); y <- (i %% 17 + 1) / 7 * 10^((i %% 5) - 2)
h(x %% y); h(x %/% y); h(-x %% y); h(x %/% -y)
h(c(1 %% 0.1, 5.5 %% 0.1, 0.3 %% 0.1, 1e15 %% 0.3, -1e10 %% 7.1, 2^60 %% 3.3, 1e17 %% 3))
x <- c(1e20, 3e20, 5) %% 3
print(x)
f <- function(a) a %/% 0.7
f(c(1e30, 2))
1e20 %% 3
g <- function(a, b) a %% b
g(1e25, 7)
k <- function() { s <- 0; for (i in 1:80) s <- s + (i * 1e18) %% 7; s }
k()
h(c(-7.5 %% 2, 7.5 %% -2, -7.5 %/% 2, Inf %% 3, 3 %% Inf, -3 %% Inf, 5 %% 0, 5 %/% 0, NA %% 2))
