# R: eval.c:applydefine evaluates a nested replacement's getter and setter
# arguments with the target still bound to its whole value (#550).
f <- function() {
  a <- list(b = 1)
  attr(a$b, { print(a); "hello" }) <- 42
  a
}
print(f())
g <- function() {
  a <- list(b = 1:3)
  a$b[{ print(a); 2 }] <- 42L
  a
}
print(g())
h <- function() {
  a <- list(b = list(c = 1:3), d = "x")
  a[[{ cat("getter\n"); print(a$b); "b" }]]$c[{ print(a$b$c); 1 }] <- 9L
  a
}
print(h())
k <- function() {
  a <- list(b = list(c = 1:3))
  attr(a$b$c, { s <- a$b$c; print(s); "nm" }) <- "v"
  a$b$c[[{ print(a$b); 2L }]] <- 7L
  a
}
print(k())
# A calling handler for a warning raised in a setter argument sees it too.
w <- function() {
  a <- list(b = 1:3)
  withCallingHandlers(
    a$b[{ warning("ww"); 2 }] <- 0L,
    warning = function(c) { print(a); invokeRestart("muffleWarning") })
  a
}
print(w())
# R code in an argument that rebinds the target: the replacement works on
# the value it fetched first, and assigns on success.
r <- function() {
  a <- list(b = 1)
  attr(a$b, { a <- list(z = 2); "hello" }) <- 42
  a
}
print(r())
r3 <- function() {
  a <- list(b = 1:3)
  try(a$b[{ a <- "gone"; stop("e") }] <- 0L, silent = TRUE)
  a
}
print(r3())
# At top level too.
a <- list(b = 1)
attr(a$b, { print(a); "hello" }) <- 42
a$b[{ print(a); 1 }] <- 5
print(a)
# The element and its container are not copied.
m <- function() {
  a <- list(b = c(1, 2, 3))
  for (i in 1:3) a$b[i] <- i * 10
  a
}
print(m())

# An outer `[[<-` evaluates its index again; one that now selects another
# element gets the modified element there.
f <- function() {
  i <- 0
  a <- list(list(c = 1:3), list(c = 4:6))
  a[[{ i <- i + 1; i }]]$c[1] <- 0L
  print(a)
}
f()
f <- function() {
  k <- 1
  a <- list(list(c = 1:3), list(c = 4:6))
  a[[k]]$c[{ k <- 2; 1 }] <- 0L
  print(a)
}
f()
f <- function() {
  a <- list(b = list(c = 1:3))
  a[[{ cat("index\n"); print(a$b$c); "b" }]]$c[2] <- 0L
  print(a)
}
f()
# Shapes the in-place path declines after the getters ran.
f <- function() {
  a <- list(b = structure(list(c = 1:3), class = "foo"))
  a[[{ cat("g\n"); "b" }]]$c[1] <- 0L
  print(unclass(a$b))
  a <- list()
  a[[{ cat("h\n"); "x" }]]$y <- 1
  print(a)
}
f()
f <- function() {
  a <- list(b = structure(1, class = "foo"))
  attr(a$b, { print(class(a$b)); "x" }) <- 1
  print(a)
}
f()
f <- function() {
  a <- list(b = 1:3)
  r <- try(a[[5]]$c <- 1, silent = TRUE)
  cat(r)
  r <- try(a$b[[{ print(a$b); 9 }]] <- list(), silent = TRUE)
  cat(r)
  print(a)
}
f()
f <- function() {
  a <- list(b = 1:3)
  lockBinding("a", environment())
  r <- try(a$b[1] <- 0L, silent = TRUE)
  cat(r)
  print(a)
}
f()
f <- function() {
  a <- list(b = list(c = list(d = 1:4)))
  a$b$c$d[{ print(a$b$c$d); 2 }] <- 0L
  a$b[["c"]]$d[3] <- 9L
  print(a)
  b <- a
  a$b$c$d[1] <- -1L
  print(b$b$c$d); print(a$b$c$d)
}
f()
