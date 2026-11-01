# Tiny closures called directly: constant, identity and NULL bodies
# (the smallest R functions; their call cost dominates such code).
n <- 60000
f1 <- function() 1
f2 <- function(x) x
f3 <- function() NULL
f4 <- function(x, y) x
f5 <- function() "v"
mk <- function(k) function() k
h <- mk(3)
s <- 0
for (i in 1:n) s <- s + f1() + f2(i) + f4(i, 2) + h() + (is.null(f3()) && nchar(f5()) == 1)
cat(s, "\n")
