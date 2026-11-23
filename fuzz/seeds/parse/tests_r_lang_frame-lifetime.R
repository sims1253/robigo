# Frames that outlive their call must hold their final variable values
mk <- function() { a <- 1; a <- a + 1; function() a }
mk()()
ex <- function() { v <- 10; on.exit(cat("on.exit sees", v, "\n")); v <- 20; v }
ex()
ge <- function() { w <- 5; w <- w * 2; environment() }
get("w", envir = ge())
sf <- function() { k <- 3; k <- k + 4; sys.frame(sys.nframe()) }
get("k", envir = sf())
da <- function() { z <- 1; delayedAssign("p", z * 100); z <- 2; p }
da()
outer <- function() { m <- 1; inner <- function() m; m <- 42; inner }
outer()()
cnt <- function() { n <- 0; list(inc = function() n <<- n + 1, get = function() n) }
c1 <- cnt(); c1$inc(); c1$inc(); c1$get()
lazyd <- function(a, b = a * 2) { a <- a + 1; b }
lazyd(1)
loopf <- function() { s <- 0; for (i in 1:3) s <- s + i; e <- environment(); e }
get("s", envir = loopf())
