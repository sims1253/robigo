# Loops in native code that index vectors with ranges and vectors, assign
# through them, and run closures with vector arguments inline (jit.rs).
# Each function is called twice (GNU R's JIT compiles small closures at
# their second call) and its loop runs long enough to be compiled.

# x[a:b] and x[v] reads
rd <- function(n) {
  x <- c(10L, 20L, 30L, 40L, 50L); d <- c(1.5, 2.5, 3.5, 4.5)
  iv <- c(2L, 2L, 5L); dv <- c(4, 1.9, 3)
  s <- 0L; t <- 0
  for (i in 1:n) {
    k <- (i %% 4L) + 1L
    a <- x[1:k]; b <- x[k:1]; c2 <- x[iv]; e <- d[dv]; f <- d[2:k]
    s <- s + a[[k]] + b[[1]] + c2[[3]]
    t <- t + e[[2]] + f[[1]]
  }
  list(s, t, a, b, c2, e, f)
}
invisible(rd(40)); print(rd(40))
# indices native code leaves for: out of range, 0, NA, fractions
ix <- function(n, j) {
  x <- c(1L, 2L, 3L); ok <- c(1, 2); jj <- j; r <- 0L
  for (i in 1:n) {
    v <- if (i == n) jj else ok
    r <- x[v]
  }
  r
}
for (j in list(c(2, 4), c(0, 1), c(NA, 1), c(0.5, 2), c(2.9, 1.1), c(3, 3, 1)))
  print(ix(40, j))
rg <- function(n, hi) {
  x <- c(5, 6, 7); r <- 0; h <- hi
  for (i in 1:n) {
    e <- if (i == n) h else 3L
    r <- x[2:e]
  }
  r
}
invisible(rg(30, 4L)); print(rg(30, 4L)); print(rg(30, 3L)); print(rg(30, 1L))
print(tryCatch(rg(30, NA), error = function(e) conditionMessage(e)))
print(rg(30, 2.5))

# assignments
wr <- function(n) {
  x <- integer(6); d <- numeric(4); iv <- c(1, 3, 3); ii <- c(1L, 4L)
  for (i in 1:n) {
    k <- (i %% 5L) + 1L
    x[1:k] <- x[k:1]
    x[2:3] <- i
    x[iv] <- x[4:6]
    d[2:4] <- i / 2
    d[ii] <- x[5:6]
  }
  list(x, d)
}
invisible(wr(41)); print(wr(41))
# perm1[1:r] <- perm1[2:(r + 1L)], the rotation of the shootout's fannkuch
rot <- function(n) {
  p <- 1:7
  for (i in 1:n) {
    r <- (i %% 5L) + 1L
    p0 <- p[[1L]]
    p[1:r] <- p[2:(r + 1L)]
    p[[r + 1L]] <- p0
  }
  p
}
invisible(rot(100)); print(rot(100))
# a vector assigned to itself, mismatched lengths, NA indices, growth,
# another type
al <- function(n) {
  x <- c(1L, 2L, 3L)
  for (i in 1:n) x[3:1] <- x
  x
}
invisible(al(31)); print(al(31)); print(al(32))
# a vector as the index into itself
xx <- function(n, v) {
  x <- c(2L, 1L, 3L); w <- c(7L, 8L, 9L); y <- 0L
  for (i in 1:n) {
    x[x] <- v
    y <- x
    y[y] <- w
  }
  list(x, y)
}
invisible(xx(30, 3L)); print(xx(30, 3L)); print(xx(30, 1L))
xg <- function(n) { x <- c(2L, 1L, 3L); for (i in 1:n) x[x] <- 9L; x }
invisible(xg(30)); print(xg(30))
mis <- function(n, v) {
  x <- c(1, 2, 3, 4); w <- v; two <- c(0, 0)
  for (i in 1:n) {
    two[[1]] <- i; two[[2]] <- -i
    x[1:2] <- if (i == 20L) w else two
  }
  x
}
invisible(mis(30, c(9, 8, 7, 6))); print(mis(30, c(9, 8, 7, 6))); print(suppressWarnings(mis(30, c(9, 8, 7))))
nai <- function(n) {
  x <- c(1, 2, 3); ok <- c(1, 2); bad <- c(1, NA); v <- c(0, 0)
  r <- tryCatch(for (i in 1:n) {
    j <- if (i == 25L) bad else ok
    v[[1]] <- i; v[[2]] <- -i
    x[j] <- v
  }, error = function(e) conditionMessage(e))
  list(r, x)
}
invisible(nai(30)); print(nai(30))
gro <- function(n) {
  x <- c(1L, 2L); h <- 2L
  for (i in 1:n) {
    if (i == 20L) h <- 4L
    x[2:h] <- i
  }
  x
}
invisible(gro(30)); print(gro(30))
ty <- function(n) {
  x <- c(1L, 2L, 3L); r <- c(0.5, 1.5)
  for (i in 1:n) x[1:2] <- if (i < 20L) i else r
  x
}
invisible(ty(30)); print(ty(30))

# range variables assigned in the loop
kr <- function(n) {
  s <- 0L; x <- c(4L, 3L, 2L, 1L)
  for (i in 1:n) {
    k_range <- 1:((i %% 4L) + 1L)
    y <- x[k_range]
    s <- s + y[[1]] + length(k_range)
  }
  list(s, k_range, typeof(k_range), y)
}
invisible(kr(50)); print(kr(50))

# closures with vector arguments, run inline
rv <- function(n) {
  p <- c(3L, 1L, 4L, 1L, 5L, 9L, 2L)
  for (i in 1:n) {
    k <- (i %% 6L) + 1L
    k_range <- 1:k
    p[k_range] <- rev.default(p[k_range])
  }
  p
}
invisible(rv(100)); print(rv(100))
em <- function(n) {
  e <- integer(0); r <- 1L
  for (i in 1:n) r <- rev.default(e)
  r
}
invisible(em(30)); print(em(30))
mid <- function(v) v[2:3]
tw <- function(v, w) if (length(v) > length(w)) v else w
cl <- function(n) {
  x <- c(1.5, 2.5, 3.5, 4.5); y <- c(9, 8, 7)
  s <- 0
  for (i in 1:n) {
    m <- mid(x)
    z <- tw(x[1:((i %% 4L) + 1L)], y)
    s <- s + m[[1]] + z[[2]]
  }
  list(s, m, z)
}
invisible(cl(40)); print(cl(40))
# rebinding what the loop relies on
rb <- function(n) {
  rev.default <- function(x) x[2:1]
  p <- c(1L, 2L, 3L)
  for (i in 1:n) p[1:2] <- rev.default(p[2:3])
  p
}
invisible(rb(30)); print(rb(30))
sb <- function(n) {
  x <- c(1, 2, 3)
  r <- 0
  for (i in 1:n) r <- x[2:3]
  `[` <- function(x, i) -1
  for (i in 1:n) r <- x[2:3]
  r
}
invisible(sb(30)); print(sb(30))
# closures native code does not run inline: another builtin, a range result
na2 <- function(v) { is.na(v); v[1:2] }
rr <- function(v) 1:length(v)
cn <- function(n) {
  x <- c(4L, 5L, 6L); s <- 0L
  for (i in 1:n) {
    a <- na2(x)
    b <- rr(x)
    s <- s + a[[2]] + length(b)
  }
  list(s, a, b)
}
invisible(cn(30)); print(cn(30))
