# `x[i, , k] <- v` and `x[i, , k] <<- v` (vm.rs Op::SetIndexN,
# Op::SuperSetIndexN): in place on plain atomic arrays for values that
# fill the slice, `[<-` as usual otherwise.
f <- function() {
  d <- array(0, c(2, 3, 4))
  for (i in 1:2) for (j in 1:3) d[i, j, ] <- c(i, j, i * j, i + j)
  print(d)
  m <- matrix(0L, 3, 4)
  m[, 2] <- 1:3; m[3, ] <- 9L; m[2, 4] <- NA; print(m)
  m[1, ] <- c(TRUE, NA); print(m)
  r <- matrix(0, 2, 6); r[, 3] <- 7L; r[2, ] <- c(NA, 1L, 2L); r[1, ] <- NA; print(r)
  l <- matrix(FALSE, 2, 2); l[, 1] <- TRUE; l[2, ] <- NA; print(l)
  # type changes, objects, attributes on the value
  m2 <- matrix(1:6, 2); m2[, 2] <- 2.5; print(m2)
  m3 <- matrix(1:6, 2); m3[1, ] <- "a"; print(m3)
  m4 <- matrix(0, 2, 2); m4[, 1] <- factor(c("u", "v")); print(m4)
  m5 <- matrix(0, 2, 2); m5[, 2] <- c(a = 1, b = 2); print(m5)
  m6 <- matrix(0, 2, 3); m6[1, ] <- matrix(1:3, 1); print(m6)
  # lengths that do not fill the slice
  m7 <- matrix(0, 4, 2); m7[, 1] <- 1:2; print(m7)
  print(tryCatch({ m7[, 2] <- 1:3; m7 }, error = function(e) conditionMessage(e)))
  print(tryCatch({ m7[, 2] <- numeric(0); m7 }, error = function(e) conditionMessage(e)))
  print(tryCatch({ m7[5, ] <- 1; m7 }, error = function(e) conditionMessage(e)))
  m8 <- matrix(0, 2, 2); print(tryCatch({ m8[, 3] <- 1; m8 }, error = function(e) conditionMessage(e)))
}
f()
# copies: another name for the same array, a global array
g <- function() {
  a <- matrix(0, 2, 2); b <- a
  for (k in 1:2) a[, k] <- k
  print(a); print(b)
  gl[, 1] <- 100
  print(gl)
  invisible()
}
gl <- matrix(1, 2, 2)
g()
print(gl)
# names, dimnames and other attributes stay
h <- function() {
  n <- matrix(0, 2, 2, dimnames = list(c("a", "b"), c("x", "y")))
  n[, 2] <- 1:2; n["a", ] <- 5; print(n)
  e <- matrix(0, 2, 2); attr(e, "extra") <- "kept"; e[1, ] <- 3; print(e)
  o <- structure(matrix(0, 2, 2), class = "bar")
  `[<-.bar` <- function(x, i, j, value) { cat("method\n"); unclass(x) }
  o[1, ] <- 1; print(o)
}
h()
# <<-
body_v <- matrix(0, 3, 5)
adv <- function(n) {
  for (k in 1:n) for (i in 1:5) body_v[, i] <<- body_v[, i] + c(i, k, 1)
}
adv(4); print(body_v)
outer_fn <- function() {
  acc <- array(0, c(2, 2, 2))
  inner <- function() for (z in 1:2) acc[, , z] <<- z * 10 + matrix(1:4, 2)
  inner(); print(acc)
  shadow <- function() {
    acc <- "local"
    acc[1] <- "still local"
    print(acc)
  }
  shadow(); print(acc[1, 1, ])
  alias <- acc
  inner2 <- function() acc[1, 1, ] <<- c(-1, -2)
  inner2(); print(acc[1, 1, ]); print(alias[1, 1, ])
}
outer_fn()
nowhere <- function() zz_missing[1, ] <<- 1
print(tryCatch(nowhere(), error = function(e) conditionMessage(e)))
locked <- function() {
  e <- new.env(); assign("q", matrix(0, 2, 2), envir = e); lockBinding("q", e)
  fn <- function() q[1, ] <<- 1
  environment(fn) <- new.env(parent = e)
  print(tryCatch(fn(), error = function(err) conditionMessage(err)))
}
locked()
tv <- function() { x <- matrix(0, 2, 2); x[, 1] <<- 1 }
print(tryCatch(tv(), error = function(e) conditionMessage(e)))
# the value of the assignment
vis <- function() { d <- array(0, c(2, 2, 2)); d[1, 1, ] <- c(5, 6) }
print(vis())
w <- matrix(0, 2, 2)
(w[, 1] <- 3)
w[2, ] <- 4
print(w)
