# Appending one element past the end in compiled code (in place)
f <- function(n) { r <- c(); for (i in 1:n) r[i] <- i; r }
f(5)
g <- function(n) { r <- numeric(0); for (i in 1:n) r[i] <- i * 0.5; r }
g(4)
h <- function() { r <- 1:3; r[4] <- 2.5; r }
h()
k <- function() { r <- c(a = 1); r[2] <- 5; r }
k()
l <- function(n) { r <- list(); for (i in 1:n) r[[length(r) + 1]] <- i; str <- vapply(r, function(z) z, 0); str }
l(4)
m <- function() { r <- c("a"); r[2] <- "b"; r[[3]] <- "c"; r }
m()
n2 <- function() { r <- TRUE; r[2] <- 1L; r }
n2()
o <- function() { r <- list(1); r[[2]] <- NULL; r[2] <- list(NULL); length(r) }
o()
p <- function() { r <- matrix(1:4, 2); r[5] <- 9L; r }
p()
q <- function() { x <- 1:3; y <- x; y[4] <- 4L; list(x, y) }
q()
w <- function() { r <- integer(0); for (i in 1:30) { r[length(r) + 1] <- i; if (i %% 10 == 0) r <- r[-1] }; r }
w()
big <- function(n) { r <- c(); for (i in 1:n) r[i] <- i * 2; c(length(r), sum(r)) }
big(5000)
lg <- function(n) { r <- list(); for (i in 1:n) r[[length(r) + 1]] <- c(i, i); length(r) }
lg(300)
shadow <- function() { length <- function(x) 0L; r <- 1:3; r[length(r) + 1] <- 9L; r }
shadow()
