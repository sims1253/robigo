# Indexing in compiled loops behaves like the interpreter.
x <- numeric(5)
for (i in 1:5) x[i] <- i * 1.5
x
v <- 1:5
for (i in 1:5) v[i] <- v[i] * 2L
v
for (i in 1:2) v[i] <- 0.5      # type change: integer -> double
v
s <- c("a", "b", "c")
for (i in 1:3) s[i] <- paste0(s[i], i)
s
l <- list(1, "a", TRUE)
for (i in 1:3) l[[i]] <- l[[i]]
for (i in 1:2) l[[i]] <- NULL   # deletion goes through the interpreter
length(l)
# copies stay independent
a <- c(1, 2, 3); b <- a
for (i in 1:3) a[i] <- 0
a; b
# out of range extends
e <- 1:2
for (i in 1:4) e[i] <- i
e
# names are kept by [<- and [ reads drop nothing / [[ drops names
n <- c(a = 1, b = 2)
for (i in 1:2) n[i] <- n[[i]] + 10
n
for (i in 1:2) print(n[i])
# matrices keep dims
m <- matrix(0, 2, 2)
for (i in 1:4) m[i] <- i
m
# classed objects dispatch
f <- factor(c("u", "v", "u"))
for (i in 1:3) print(f[i])
`[.myc` <- function(x, i) { cat("my [ with", i, "\n"); unclass(x)[i] * 100 }
o <- structure(1:3, class = "myc")
for (i in 1:2) print(o[i])
`[<-.myc` <- function(x, i, value) { cat("my [<- at", i, "\n"); y <- unclass(x); y[i] <- value; structure(y, class = "myc") }
for (i in 1:2) o[i] <- 9L
unclass(o)
# arguments: modifying a parameter copies
g <- function(z) { for (i in seq_along(z)) z[i] <- z[i] + 1; z }
w <- c(1, 2)
g(w); w
# double and NA indices
y <- c(10, 20, 30)
for (k in c(1.9, 3)) print(y[k])
r <- tryCatch({ for (k in c(1, NA)) print(y[k]) ; "ok" }, error = function(e) conditionMessage(e))
r
# errors report the right call
h <- function() { q <- 1:3; for (i in 1:5) q[[i + 0]] ; q }
tryCatch(h(), error = function(e) print(conditionCall(e)))
z <- 1:3
tryCatch(for (i in 4) z[[i]], error = function(e) print(conditionCall(e)))
# a global read inside a function, assigned locally
gv <- c(5, 6)
k2 <- function() { for (i in 1:2) gv[i] <- 0; gv }
k2(); gv
# the loop variable vector modified inside the loop
it <- 1:3
for (i in it) it[i] <- 0L
it
