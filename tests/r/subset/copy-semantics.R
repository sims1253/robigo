# Replacement must not affect other references to the same value, while
# loops of `x[i] <- v` must still be fast (modified in place).
a <- c(1, 2, 3)
b <- a
b[1] <- 100
a
b
l1 <- list(p = list(q = 1))
l2 <- l1
l2$p$q <- 2
l1$p$q
l2$p$q
f <- function(v) {
  v[2] <- -1
  v
}
orig <- 1:3
f(orig)
orig
g <- function(v) {
  v[[1]] <- "changed"
  v$extra <- TRUE
  v
}
lst <- list("x")
str_res <- g(lst)
str_res
lst
h <- function(v, i) {
  v[i] <- 0L
  v
}
h(1:4, 2:3)
h(1:4)
# the index may be the vector itself
p <- c(3L, 1L, 2L)
dim(p) <- c(3L, 1L)
p[p, 1] <- 1:3
as.vector(p)
s <- 1:3
s[s] <- s * 10L
s
self <- list(1, 2)
self[[2]] <- self
length(self[[2]])
self[[2]][[2]]
# environments are references
e1 <- new.env()
e2 <- e1
e2$v <- "shared"
e1$v
# a large loop runs in place
n <- 50000L
big <- numeric(n)
for (i in seq_len(n)) big[i] <- i
big[n]
grow <- integer(0)
for (i in 1:20000) grow[i] <- i
length(grow)
lst2 <- vector("list", 10000)
for (i in seq_along(lst2)) lst2[[i]] <- i
lst2[[10000]]
cnt <- c(a = 0)
for (i in 1:20000) cnt["a"] <- cnt["a"] + 1
cnt
m <- numeric(10000)
dim(m) <- c(100L, 100L)
for (i in 1:100) for (j in 1:100) m[i, j] <- i * j
m[100, 100]
