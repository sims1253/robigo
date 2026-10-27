x <- c(a = 1.5, b = -2, c = NA)
x + 1; 1 + x; x * x; x - c(1, 2, 3); x / 0; x %% 2; x %/% 2; x^2; 2^x
y <- 1:6 / 2; y + 1:2; y * c(10, 20, 30)
z <- (y * 2 + 1) / (1 + abs(y - 3)); z
m <- matrix(1:6 / 4, 2); m + 1; m * m
sqrt(c(4, 9, -1)); sqrt(c(x = 4)); exp(1:3 / 2); sqrt(m)
v <- c(1.5, NA, NaN); v + 1; sqrt(v); is.na(sqrt(v))
w <- (1:5) / 3; w2 <- w; w3 <- w2 * 2; w; w3
s <- 0; for (k in 1:3) { q <- (1:4) * k + 0.5; s <- s + sum(sqrt(q)) }; s
