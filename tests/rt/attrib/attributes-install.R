# attributes<- and attr<- install their names (installTrChar): names the
# session has, new ones, and names in another encoding
x <- 1:3
attributes(x) <- list(names = c("a", "b", "c"), dim = 3L, someNewAttribute = "v")
attributes(x)
y <- 1
attributes(y) <- list(class = "foo", thisNameIsNewHere = TRUE)
attributes(y)
unclass(y)
z <- 1
a <- list(1)
names(a) <- "café"
attributes(z) <- a
attributes(z)
attr(z, "naïve") <- 2
names(attributes(z))
l1 <- "caf\xe9"
Encoding(l1) <- "latin1"
w <- 1
attr(w, l1) <- 3
names(attributes(w))
identical(names(attributes(w)), "café")
attributes(w) <- NULL
w
attr(w, "dim") <- 1L
attributes(w)
