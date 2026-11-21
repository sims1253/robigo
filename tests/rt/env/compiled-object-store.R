# A compiled local assignment of an object (SETVAR into a closure frame's
# slot, robigo-rt's frame_store_object_raw) does what defineVar does: the
# binding's order (ls(sorted = FALSE) by definition), a locked binding's
# error, a function-valued local seen by the next lookup of a call, an S3
# method's argument (aliased to its generic's), a reassigned formal and
# missing(), each in the closure's first call and later (byte-code) ones.
f1 <- function() {
  b <- list(1); a <- "x"; c <- c(1, 2); b <- list(2)
  ls(sorted = FALSE)
}
print(f1()); print(f1())
f2 <- function() {
  v <- c(1, 2)
  lockBinding("v", environment())
  r <- tryCatch({ v <- c(3, 4); "stored" }, error = conditionMessage)
  list(r, v)
}
print(f2()); print(f2())
f3 <- function() {
  g <- function() "first"
  r1 <- g()
  g <- function() "second"
  c(r1, g())
}
print(f3()); print(f3())
area <- function(s, ...) UseMethod("area")
area.sq <- function(s, ...) { s <- unclass(s); s$side <- s$side * 2; s$side^2 }
sq <- structure(list(side = 2), class = "sq")
print(area(sq)); print(area(sq)); print(sq$side)
f4 <- function(x, y) {
  m <- missing(y)
  x <- list(x)
  y <- "now"
  list(m, missing(y), x, y)
}
print(f4(1)); print(f4(1)); print(f4(1, 2))
f5 <- function() {
  v <- c(1, 2, 3)
  w <- v
  v <- c(v, 4)
  list(v, w)
}
print(f5()); print(f5())
f6 <- function(n) {
  acc <- character(0)
  for (i in seq_len(n)) acc <- c(acc, letters[i])
  e <- environment()
  list(acc, ls(e, sorted = FALSE))
}
print(f6(3)); print(f6(3))
