# Model fitting: lm and glm on 20k observations, repeated.
set.seed(7)
n <- 20000
x1 <- sin(1:n); x2 <- cos((1:n) / 3); g <- factor((1:n) %% 4)
y <- 1 + 2 * x1 - x2 + as.integer(g) / 3 + sin((1:n) * 7)
b <- (y > median(y)) + 0
s <- 0
for (k in 1:5) {
  f <- lm(y ~ x1 + x2 + g)
  s <- s + sum(coef(f))
}
s <- s + sum(coef(glm(b ~ x1 + x2, family = binomial)))
cat(sprintf("%.8f", s), "\n")
