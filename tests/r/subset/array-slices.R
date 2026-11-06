# `x[i, , k]`: slices of arrays by scalar and empty indices (vm.rs
# Op::IndexN): plain atomic arrays are read directly, anything else is
# `[` as usual. Run in functions (compiled) and loops.
m <- matrix(as.numeric(1:15), 3, 5)
d <- array(1:60, c(3, 4, 5))
f <- function() {
  print(m[, 2]); print(m[2, ]); print(m[3, 5]); print(m[, 5])
  print(d[2, 3, ]); print(d[, 4, 5]); print(d[1, , 2]); print(d[, , 3]); print(d[2, , ])
  print(d[, 1, ]); print(d[3, 4, 5]); print(d[2.9, 1, 1])
  l <- array(c(TRUE, NA, FALSE, TRUE), c(2, 2)); print(l[, 2]); print(l[1, ])
  print(dim(d[, , 1])); print(dim(d[1, , ]))
  r1 <- matrix(c(1, 2, 3), 1); print(r1[1, ]); print(r1[, 2]); print(r1[, 1:2])
  c1 <- matrix(c(1, 2, 3), 3); print(c1[, 1]); print(c1[2, ])
  a1 <- array(7, c(1, 1, 1)); print(a1[1, 1, ]); print(a1[, , 1])
  e <- array(1:24, c(2, 3, 1, 4)); print(e[1, , 1, ]); print(e[, 2, , 3])
  s <- 0
  for (j in 1:5) for (i in 1:3) s <- s + sum(m[, j]) * m[i, ][[j]]
  print(s)
}
f()
# names, other attributes, other types, objects: `[` itself
g <- function() {
  n <- m; dimnames(n) <- list(c("a", "b", "c"), NULL); print(n[, 2]); print(n[2, ])
  k <- matrix(letters[1:6], 2); print(k[, 3]); print(k[2, ])
  z <- matrix(complex(real = 1:4, imaginary = 1), 2); print(z[, 2])
  li <- matrix(list(1, "a", TRUE, NULL), 2); print(li[, 2])
  df <- data.frame(a = 1:3, b = c(2.5, 3.5, 4.5)); print(df[, 2]); print(df[2, ])
  tm <- structure(m, class = "foo")
  `[.foo` <- function(x, i, j, ...) "method"
  print(tm[, 2])
  at <- m; attr(at, "extra") <- 1; print(at[, 2])
  em <- matrix(numeric(0), 0, 3); print(em[, 2]); print(dim(em[, 2:3]))
}
g()
# indices: vectors, logicals, NA, zero, negative, out of range, characters
h <- function() {
  j <- c(1, 3); print(m[, j]); t <- TRUE; print(m[t, 2]); na <- NA_integer_; print(m[na, 2])
  z <- 0; print(m[, z]); neg <- -1; print(m[neg, 2])
  big <- 6; print(tryCatch(m[, big], error = function(e) conditionMessage(e)))
  big <- 4; print(tryCatch(d[big, 1, ], error = function(e) conditionMessage(e)))
  print(tryCatch(m[, nope], error = function(e) conditionMessage(e)))
  print(tryCatch(d[1, ], error = function(e) conditionMessage(e)))
  print(tryCatch(d[1, 2, 3, ], error = function(e) conditionMessage(e)))
  v <- 1:5; print(tryCatch(v[1, ], error = function(e) conditionMessage(e)))
  i <- 2L; print(d[i, i, ]); x <- 2.5; print(d[x, x, ])
}
h()
# rebinding `[`, a promise as the array, the drop argument
k <- function(a, i) {
  print(a[i, ])
  `[` <- function(x, ...) "mine"
  print(a[i, ])
}
k(m, 2)
p <- function() { print(m[1, , drop = FALSE]); print(d[1, 1, , drop = FALSE]) }
p()
# at top level
m[, 3]
d[3, , 5]
invisible(m[1, ])
