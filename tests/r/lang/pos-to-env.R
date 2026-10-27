h <- function() exists("qq")
k <- function() { qq <- 1; h() }
k()
h2 <- function() environmentName(as.environment(-1))
h2()
f3 <- function() { e <- environment(); identical(as.environment(-1), e) }
f3()
g4 <- function(x = as.environment(-1)) x
f4 <- function() { e <- environment(); identical(g4(), e) }
f4()
