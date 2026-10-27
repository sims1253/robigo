# Common vector idioms: logical subsetting, ifelse, cumsum, which, rev, order.
x <- sin(1:1e6)
r <- 0
for (k in 1:5) {
  pos <- x[x > 0]
  r <- r + length(pos) + sum(pos)
  y <- ifelse(x > 0.5, x, -x)
  r <- r + sum(y)
  r <- r + sum(cumsum(x) > 0)
  r <- r + sum(which(x < -0.9))
  o <- order(x)
  r <- r + o[1] + o[length(o)]
}
cat(sprintf("%.6f", r), "\n")
