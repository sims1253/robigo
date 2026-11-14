# `dim<-` and `dimnames<-`: the dimnames go with a new dim, the names
# too; dimnames are a list of character vectors (or NULL)
x <- 1:6
names(x) <- c("a", "b", "c", "d", "e", "f")
dim(x) <- 2:3
attributes(x)
dim(x) <- NULL
attributes(x)
dim(x) <- c(3, 2)
attributes(x)
m <- 1:4
dim(m) <- c(2L, 2L)
dimnames(m) <- list(c("r1", "r2"), c("c1", "c2"))
attributes(m)
dimnames(m) <- list(NULL, 1:2)
attributes(m)
dimnames(m) <- list(c("r1", "r2"))
attributes(m)
dimnames(m) <- list(a = character(0), b = c(TRUE, FALSE))
attributes(m)
dimnames(m) <- NULL
attributes(m)
dim(m) <- c(2L, 2L)
attributes(m)
a <- 1:3
dim(a) <- 3L
names(a) <- c("i", "j", "k")
attributes(a)
names(a)
names(a) <- NULL
attributes(a)
p <- pairlist(1, 2)
dim(p) <- 2L
dimnames(p) <- list(c("x", "y"))
p
l <- list(1, 2, 3, 4)
dim(l) <- c(2L, 2L)
attributes(l)
