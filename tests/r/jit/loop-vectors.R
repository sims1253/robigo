# Loops in native code with vector and range variables, max/min of scalars,
# numeric conditions, and ops native code leaves at (jit.rs, vm::jit_enter).
# Every loop runs long enough to be compiled.

# A vector assigned in the loop is the loop's own copy.
cp <- function(n) {
  p <- c(3L, 1L, 2L)
  q <- integer(0)
  for (i in 1:n) {
    q <- p
    q[[1]] <- i
    p[[2]] <- p[[2]] + 1L
  }
  list(p, q)
}
invisible(cp(40)); print(cp(40)); invisible(cp(1)); print(cp(1))
cpd <- function(n) {
  x <- c(0.5, 1.5)
  y <- 0
  for (i in 1:n) {
    y <- x
    y[[2]] <- y[[2]] * 2
    x[[1]] <- x[[1]] + y[[2]]
  }
  list(x, y)
}
invisible(cpd(30)); print(cpd(30))
# growing and shrinking, and assigning a vector to itself
gr <- function(n) {
  a <- 1:2; b <- c(5L, 6L, 7L, 8L, 9L); z <- a
  for (i in 1:n) {
    z <- if (i %% 2L == 0L) b else a
    z <- z
    z[[1]] <- i
  }
  z
}
invisible(gr(41)); print(gr(41)); invisible(gr(42)); print(gr(42))
# read before written: the value at entry
re <- function(n) {
  b <- c(1L, 2L)
  for (i in 1:n) {
    c2 <- b
    b <- c2
    b[[1]] <- b[[1]] + i
  }
  b
}
invisible(re(50)); print(re(50))
# continuing in the VM: the copy so far is the variable's value
dv <- function(n) {
  p <- c(1L, 2L, 3L)
  for (i in 1:n) {
    q <- p
    q[[2]] <- i
    if (i == 25L) q[[5]] <- 9L
    p[[1]] <- q[[2]]
  }
  list(p, q)
}
invisible(dv(30)); print(dv(30))
ov <- function(n) {
  p <- c(1L, 2147483600L)
  for (i in 1:n) {
    q <- p
    p[[2]] <- q[[2]] + 1L
  }
  list(p, q)
}
invisible(suppressWarnings(ov(60))); print(suppressWarnings(ov(60)))

# Range variables.
rv <- function(n) {
  rr <- 3:7
  s <- 0L
  for (i in 1:n) {
    r2 <- rr
    for (j in r2) s <- s + j
  }
  list(s, r2, typeof(r2), length(r2))
}
invisible(rv(20)); print(rv(20))
rd <- function(n) {
  rr <- 10:6
  s <- 0L
  for (i in 1:n) for (j in rr) s <- s + j * i
  s
}
invisible(rd(30)); print(rd(30))
le <- function(n) {
  v <- c(2, 4, 6); rr <- 1:4
  s <- 0L
  for (i in 1:n) s <- s + length(v) + length(rr)
  s
}
invisible(le(40)); print(le(40))

# max and min of scalars
mm <- function(n) {
  a <- 0L; b <- 100L; d <- -Inf; e <- Inf
  for (i in 1:n) {
    a <- max(a, i %% 7L)
    b <- min(b, 100L - i)
    d <- max(d, i / 3, -i)
    e <- min(e, i * 0.5, 50L)
  }
  list(a, b, d, e, typeof(a), typeof(d))
}
invisible(mm(60)); print(mm(60))
# (Ties keep the first; zeros come in as arguments, as GNU R's compiler
# keeps one constant for 0 and -0 in a function.)
mz <- function(n, nz, pz) {
  z <- pz; for (i in 1:n) z <- max(nz, z); w <- nz; for (i in 1:n) w <- max(w, pz)
  u <- pz; for (i in 1:n) u <- min(u, nz)
  c(1 / z, 1 / w, 1 / u)
}
invisible(mz(30, -0, 0)); print(mz(30, -0, 0))
ml <- function(n) {
  k <- 0L; for (i in 1:n) k <- max(TRUE, k, FALSE)
  list(k, typeof(k))
}
invisible(ml(30)); print(ml(30))
mn <- function(n, x) {
  r <- 0L
  for (i in 1:n) r <- max(r, if (i == 20L) x else i)
  r
}
invisible(mn(40, NA_integer_)); print(mn(40, NA_integer_)); invisible(mn(40, NA_real_)); print(mn(40, NA_real_)); invisible(mn(40, NaN)); print(mn(40, NaN)); invisible(mn(40, 7.5)); print(mn(40, 7.5))
mr <- function(n) {
  max <- function(...) -1
  r <- 0
  for (i in 1:n) r <- max(r, i)
  r
}
invisible(mr(30)); print(mr(30))

# Integer and double conditions
cd <- function(n) {
  k <- n; s <- 0
  while (k) { s <- s + k; k <- k - 1L }
  t <- 0
  x <- 2.5
  for (i in 1:n) if (x - (i %% 3)) t <- t + 1
  c(s, t)
}
invisible(cd(40L)); print(cd(40L))
cn <- function(n, bad) {
  s <- 0L
  for (i in 1:n) { if (if (i == 30L) bad else i) s <- s + 1L }
  s
}
invisible(tryCatch(cn(40, NA), error = function(e) conditionMessage(e)))
print(tryCatch(cn(40, NA), error = function(e) conditionMessage(e)))
invisible(tryCatch(cn(40, NA_integer_), error = function(e) class(e)))
print(tryCatch(cn(40, NA_integer_), error = function(e) class(e)))
invisible(tryCatch(cn(40, NaN), error = function(e) class(e)))
print(tryCatch(cn(40, NaN), error = function(e) class(e)))
invisible(cn(40, 0)); print(cn(40, 0))

# Ops native code does not have: it leaves there, the VM goes on.
pr <- function(n) {
  s <- 0L; v <- c(1L, 2L)
  for (i in 1:n) {
    s <- s + i
    v[[1]] <- s
    if (i %% 25L == 0L) cat("at", i, "s =", s, "v =", v, "\n")
  }
  s
}
invisible(pr(100)); print(pr(100))
rt <- function(n) {
  p <- c(5L, 6L)
  for (i in 1:n) {
    q <- p
    q[[2]] <- i
    if (i == 33L) return(list(i, q, p))
  }
  "not reached"
}
invisible(rt(50)); print(rt(50))
ev <- function(n) {
  acc <- character(0); s <- 0
  for (i in 1:n) {
    s <- s + i
    acc <- paste0("x", s)
  }
  list(s, acc)
}
invisible(ev(30)); print(ev(30))
st <- function(n) {
  s <- 0
  for (i in 1:n) {
    s <- s + 1
    if (i == 40) stop("stopped at ", i, " with s = ", s)
  }
  s
}
invisible(tryCatch(st(60), error = function(e) conditionMessage(e)))
print(tryCatch(st(60), error = function(e) conditionMessage(e)))
wn <- function(n) {
  s <- 0L
  for (i in 1:n) { s <- s + 1L; if (i == 35L) warning("w at ", s) }
  s
}
invisible(withCallingHandlers(wn(50), warning = function(w) { cat("caught:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }))
print(withCallingHandlers(wn(50), warning = function(w) { cat("caught:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }))

# The shootout's fannkuch, written with element swaps.
fannkuch <- function(n) {
  if (n > 3L) rxrange = 3:(n - 1) else rxrange = integer(0)
  max_flip_count <- 0L
  perm_sign <- TRUE
  checksum <- 0L
  perm1 <- 1:n
  count <- 0:(n - 1L)
  while (TRUE) {
    if (k <- perm1[[1L]]) {
      perm <- perm1
      flip_count <- 1L
      while ((kk <- perm[[k]]) > 1L) {
        for (lo in 1:(k %/% 2L)) {
          hi = k - lo + 1L
          t <- perm[[lo]]; perm[[lo]] <- perm[[hi]]; perm[[hi]] <- t
        }
        flip_count <- flip_count + 1L
        k <- kk
        kk <- perm[[kk]]
      }
      max_flip_count <- max(max_flip_count, flip_count)
      checksum <- checksum + if (perm_sign) flip_count else -flip_count
    }
    if (perm_sign) {
      t <- perm1[[1]]; perm1[[1]] <- perm1[[2]]; perm1[[2]] <- t
      perm_sign = FALSE
    } else {
      t <- perm1[[2]]; perm1[[2]] <- perm1[[3]]; perm1[[3]] <- t
      perm_sign = TRUE
      was_break <- FALSE
      for (r in rxrange) {
        if (count[[r]]) {
          was_break <- TRUE
          break
        }
        count[[r]] <- r - 1L
        perm0 <- perm1[[1L]]
        for (i in 1:r) perm1[[i]] <- perm1[[i + 1L]]
        perm1[[r + 1L]] <- perm0
      }
      if (!was_break) {
        r <- n
        if (!count[[r]]) {
          cat(checksum, "\n", sep = "")
          return(max_flip_count)
        }
      }
      count[[r]] <- count[[r]] - 1L
    }
  }
}
for (n in 5:7) print(fannkuch(n))

# Visibility when native code leaves at a return: as the VM would have it.
f <- function() { invisible(1); for (i in 1:30) { s <- i; if (i == 25L) return(s) } }
f(); f(); f()
g <- function() { a <- c(1L, 2L); for (i in 1:30) { b <- a; b[[1]] <- i; if (i == 25L) return(b) } }
g(); g()
h <- function() { for (i in 1:30) { s <- max(i, 3L); if (i == 25L) return(invisible(s)) } }
h(); h(); print(h())
k <- function() { s <- 0; for (i in 1:30) { s <- s + i; if (i == 28L) break }; s }
k(); k()

# Leaving at a call inside an expression: the VM goes on with the registers
# native code computed before it.
f <- function(n) {
  x <- c(1L, 2L, 3L)
  for (i in 1:n) {
    x[[1]] <- i
    if (i == 25L) return(sum(x, sum(1L, 2L)))
  }
}
invisible(f(30)); print(f(30))
g <- function(n) {
  x <- c(1.5, 2.5)
  for (i in 1:n) {
    y <- x
    y[[2]] <- i
    if (i == 25L) return(c(y, max(1L, sum(2L))))
  }
}
invisible(g(30)); print(g(30))
