# Sorting and ordering: sorted, reversed, random and tied inputs.
set.seed(1)
xs <- sort(sample(30000))
xr <- rev(xs)
xu <- sample(30000)
xd <- sin(1:30000) * 1e5
s <- 0L
for (k in 1:20) {
  s <- s + order(xs)[1] + order(xr)[1] + order(xu)[1]
  s <- s + sum(order(xu, decreasing = TRUE)[1:3])
  s <- s + sort(xs)[1] + sort(xr)[1] + sort(xu)[1] + sort(xu)[30000]
  s <- s + order(xd)[1]
}
t <- rep(1:100, 100)
for (k in 1:10) s <- s + order(t)[1] + sort(t)[1]
m1 <- rep(1:50, each = 100)
m2 <- sample(10000, 5000, replace = TRUE)
for (k in 1:10) s <- s + order(m1, m2)[1]
f <- factor(sample(letters[1:8], 5000, replace = TRUE))
for (k in 1:10) s <- s + order(f)[1]
cat(s, "\n")
