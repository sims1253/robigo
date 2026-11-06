# A loop in native code calls a closure inline that reads variables of its
# own environment: scalars, and elements of vectors and matrices
# (vm::inline_call, jit.rs `InlineCall::free`). They are read when the
# loop starts, from the instance called, and stay as they were while it
# runs; anything else is the call as usual.
outer <- function(n) {
  cache <- matrix(0, n, n)
  for (i in 1:n) for (j in 1:n) cache[[i, j]] <- 1 / ((i + j - 2) * (i + j - 1) / 2 + i)
  k <- 0.5
  get <- function(i, j) cache[[i, j]] * k
  lin <- function(i) cache[i] + k
  use <- function() {
    m <- matrix(0, n, n)
    s <- 0
    for (i in 1:n) for (j in 1:n) {
      m[[i, j]] <- get(i, j)
      s <- s + lin(i + (j - 1) * n)
    }
    c(sum(m), s)
  }
  r <- use()
  for (t in 1:3) r <- r + use()
  r
}
print(outer(20)); print(outer(7)); print(outer(1))
# a free vector of integers, and a free scalar of each type
iv <- function() {
  w <- c(3L, 1L, 4L, 1L, 5L, 9L, 2L, 6L); off <- 10L; flag <- TRUE
  at <- function(i) w[[i]] + off
  run <- function() { s <- 0L; for (i in 1:8) for (r in 1:4) s <- s + at(i); s }
  print(run())
  off <- 1L; print(run())
  w[3] <- NA; print(run())
}
iv()
# out of range, not numeric, an object, changed by the loop, missing
oddities <- function() {
  v <- c(1, 2, 3)
  f <- function(i) v[[i]]
  g <- function() { s <- 0; for (i in 1:20) s <- s + f((i - 1) %% 3 + 1); s }
  print(g())
  h <- function() { s <- 0; for (i in 1:20) s <- s + f(i); s }
  print(tryCatch(h(), error = function(e) conditionMessage(e)))
  v <- letters; print(tryCatch(g(), error = function(e) conditionMessage(e)))
  v <- structure(c(1, 2, 3), class = "myc")
  `[[.myc` <- function(x, i) 100
  print(g())
  v <- c(1, 2, 3)
  mod <- function() { s <- 0; for (i in 1:20) { v[[1]] <- i; s <- s + f(1) }; s }
  print(mod())
  rm(v)
  print(tryCatch(g(), error = function(e) conditionMessage(e)))
}
oddities()
# instances that see different values; `[[` rebound where the closure lives
inst <- function(scale) {
  tab <- c(1, 2, 3) * scale
  f <- function(i) tab[[i]]
  s <- 0
  run <- function() for (i in 1:30) s <<- s + f((i - 1) %% 3 + 1)
  run()
  s
}
print(inst(1)); print(inst(10)); print(inst(0.5))
reb <- function() {
  `[[` <- function(x, i) -1
  tab <- c(1, 2, 3)
  f <- function(i) tab[[i]]
  s <- 0
  for (i in 1:30) s <- s + f(2)
  s
}
print(reb())
# a closure defined in the loop's own frame
same <- function() {
  tab <- c(5, 6, 7)
  f <- function(i) tab[[i]]
  s <- 0
  for (i in 1:30) s <- s + f(3)
  s
}
print(same())
# a closure whose environment encloses the loop's frame: the loop's
# pending assignments are what it must read
encl <- function() {
  x <- 1
  f <- (function() function(i) x + i)()
  out <- 0
  for (j in 1:3) {
    x <- j
    s <- 0
    for (i in 1:100) s <- s + f(i)
    out <- out + s
  }
  out
}
print(encl())
# the loop assigns the free variable with <<- (where native loops do that)
k <- 10
addk <- function(x) x + k
sup <- function() { s <- 0; for (i in 1:40) { if (i == 20) k <<- 1000; s <- s + addk(i) }; s }
print(sup()); print(k)
