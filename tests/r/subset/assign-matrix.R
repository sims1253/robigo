# `[<-` and `[[<-` on matrices and arrays.
m <- numeric(6)
dim(m) <- c(2L, 3L)
m[1, 2] <- 5
m
m[2, ] <- c(7, 8, 9)
m
m[, 3] <- 0
m
m[] <- 1:6
m
m[c(TRUE, FALSE), ] <- -1
m
m[-1, -1] <- c(10, 20)
m
m[[2, 1]] <- 99
m
m[5] <- 50
m
m[NA, 1] <- 0
m
m[1, 1] <- "a"
m
dimnames(m) <- list(c("r1", "r2"), c("A", "B", "C"))
m["r2", "B"] <- "z"
m
m[, c("C", "A")] <- c("c1", "c2", "a1", "a2")
m
m[["r1", "C"]] <- "q"
m
# matrix subscripts in assignment
k <- integer(9)
dim(k) <- c(3L, 3L)
ij <- c(1L, 2L, 3L, 1L, 2L, 3L)
dim(ij) <- c(3L, 2L)
k[ij] <- 1:3
k
kc <- k
dimnames(kc) <- list(c("a", "b", "c"), c("x", "y", "z"))
ijc <- c("a", "c", "z", "x")
dim(ijc) <- c(2L, 2L)
kc[ijc] <- -1L
kc
# list matrix
lm <- vector("list", 4)
dim(lm) <- c(2L, 2L)
lm[1, 2] <- list("hello")
lm[[2, 2]] <- 1:3
lm[[1, 2]]
lm[[2, 2]]
# arrays
a <- integer(8)
dim(a) <- c(2L, 2L, 2L)
a[1, 2, 2] <- 5L
a[2, , ] <- 1:4
a[, , 1] <- a[, , 1] + 10L
a
a[[2, 2, 2]] <- 0L
a[, 1, 1]
a[a > 10] <- -1L
as.vector(a)
# assigning outside the dims extends a vector and drops dims
e <- 1:4
dim(e) <- c(2L, 2L)
e[6] <- 6L
e
attributes(e)
# promotion keeps the dims
p <- 1:4
dim(p) <- c(2L, 2L)
p[1, 1] <- 0.5
p
p[2, 2] <- TRUE
p
