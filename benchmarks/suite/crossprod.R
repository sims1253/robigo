# Cross products: crossprod(X) and tcrossprod(X) (symmetric, dsyrk in
# GNU R), the two-argument forms, X'y, and many small products.
x <- matrix(sin(1:240000) * 10^((1:240000) %% 5 - 2), 600, 400)
s <- crossprod(x)
t <- tcrossprod(x[1:300, ])
u <- crossprod(x[, 1:200], x[, 201:400])
v <- tcrossprod(x[1:200, ], x[201:400, ])
w <- crossprod(x, x[, 7])
q <- 0
for (k in 1:3000) {
  m <- x[k %% 500 + 1:20, k %% 300 + 1:6]
  q <- q + sum(crossprod(m)) + sum(tcrossprod(m[1:5, ]))
}
cat(sprintf("%a", c(sum(s), sum(t), sum(u), sum(v), sum(w), q)), "\n")
