# `[` and `[[` on atomic vectors: index kinds, names, NA and out of bounds.
x <- c(a = 10, b = 20, c = 30, d = 40)
x[2]
x[c(1, 3)]
x[-1]
x[c(-1, -4)]
x[-(1:4)]
x[0]
x[c(0, 2, 0)]
x[c(-2, 0)]
x[c(TRUE, FALSE)]
x[c(TRUE, NA)]
x[c(TRUE, FALSE, TRUE, FALSE, TRUE, TRUE)]
x[x > 15]
x[NA]
x[NA_integer_]
x[c(1, NA, 3)]
x[5]
x[c(2, 7)]
x[2.9]
x[c(1.5, 3.99)]
x[-1.7]
x[c("b", "d")]
x[c("b", "zz", NA, "")]
x[]
x[NULL]
x[[3]]
x[["c"]]
x[[2.5]]
x[integer(0)]
x[character(0)]
y <- 1:10
y[y %% 2L == 0L]
y[-length(y)]
y[c(10, 1)]
y[11:12]
l <- c(TRUE, NA, FALSE)
l[2:3]
l[[2]]
s <- c("u", "v", "w")
s[c(3, 3, 1)]
s[[1]]
cp <- c(1+2i, 3-1i)
cp[2]
r <- as.raw(c(1, 255, 16))
r[2:3]
r[5]
r[[3]]
z <- 1:3
z[c(-1, 0)]
# names are kept, other attributes dropped
w <- c(p = 1, q = 2)
attr(w, "extra") <- "yes"
w[1]
attributes(w[2:1])
attributes(w[])
# factor index uses the codes
fi <- c(2L, 1L)
attr(fi, "levels") <- c("x", "y")
class(fi) <- "factor"
x[fi]
# zero-length and NULL vectors
e <- numeric(0)
e[1]
e[[1]] <- 5
e
NULL[1]
NULL[[1]]
NULL$a
nn <- NULL
nn[3]
length(x[10:12])
