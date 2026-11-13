# `[` and `[[` on integer, double and character vectors, and their setters
x <- 1:10
x[3]
x[-1]
x[x > 7]
x[x %% 2L == 1L]
x[12]
x[[4]]
x[2] <- 20L
x
x[12] <- 0L
x
x[3] <- 2.5
x
y <- x * 2
y[[1]] <- -1
y[1:3]
s <- "a"
s[3] <- "c"
s
s[[2]] <- "b"
s
s[3:1]
