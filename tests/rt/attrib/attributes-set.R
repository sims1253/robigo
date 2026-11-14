# `attributes<-`: the list replaces every attribute, `dim` set first so
# that `dimnames` finds it
z <- c(a = 1, b = 2)
attributes(z) <- NULL
z
attributes(z) <- list(names = c("p", "q"), foo = 1)
attributes(z)
attributes(z) <- attributes(z)
attributes(z)
w <- 1:4
attributes(w) <- list(dimnames = list(c("a", "b"), NULL), dim = c(2L, 2L))
attributes(w)
k <- 1:3
attributes(k) <- list(class = "foo")
oldClass(k)
attributes(k) <- list()
attributes(k)
p <- pairlist(a = 1, b = 2)
attributes(p) <- NULL
p
n <- NULL
attributes(n) <- NULL
n
attributes(n) <- list(a = 1)
n
