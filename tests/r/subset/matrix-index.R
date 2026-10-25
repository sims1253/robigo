# `[` and `[[` on matrices and arrays: drop, dimnames, matrix subscripts.
m <- 1:12
dim(m) <- c(3L, 4L)
m
m[2, 3]
m[2, ]
m[, 3]
m[2, , drop = FALSE]
m[, 3, drop = FALSE]
m[c(1, 3), c(2, 4)]
m[-1, ]
m[c(TRUE, FALSE, TRUE), 4]
m[, c(TRUE, FALSE)]
m[NA, 1]
m[c(1, NA), 2:3]
m[0, ]
m[, 0]
m[5]
m[m > 8]
m[]
m[[2, 4]]
m[[7]]
dimnames(m) <- list(rows = c("a", "b", "c"), cols = c("w", "x", "y", "z"))
m
m["b", ]
m[, "y"]
m["c", "z"]
m[c("a", "c"), c("x", "x")]
m[["b", "w"]]
m[2:3, "x", drop = FALSE]
m[1, , drop = TRUE]
m[1, 1, drop = FALSE]
m[6]
m[c(1, 12)]
# matrix subscripts
idx <- c(1L, 3L, 2L, 4L)
dim(idx) <- c(2L, 2L)
m[idx]
idr <- c(1, 2, 1, 1)
dim(idr) <- c(2L, 2L)
m[idr]
ids <- c("a", "c", "x", "z")
dim(ids) <- c(2L, 2L)
m[ids]
idz <- c(1L, 0L, 2L, 3L)
dim(idz) <- c(2L, 2L)
m[idz]
idn <- c(1L, NA, 2L, 3L)
dim(idn) <- c(2L, 2L)
m[idn]
# arrays
a <- 1:24
dim(a) <- c(2L, 3L, 4L)
a[2, 3, 4]
a[1, , ]
a[, 2, ]
a[, , 1]
a[1, 2, ]
a[2, 3, 4, drop = FALSE]
a[, 1:2, 3:4]
dim(a[, 1, , drop = FALSE])
a[[1, 2, 3]]
dimnames(a) <- list(c("p", "q"), NULL, c("k1", "k2", "k3", "k4"))
a[, , "k2"]
a["q", 1, ]
a[, 2, 2:3]
# 1-d arrays keep their dim unless the result has length 1
v <- c(5L, 6L, 7L)
dim(v) <- 3L
dimnames(v) <- list(c("i", "j", "k"))
v[2:3]
v[2]
v[2, drop = FALSE]
attributes(v[c("k", "i")])
# list matrices
lm <- list(1, "a", TRUE, NULL)
dim(lm) <- c(2L, 2L)
lm[2, 1]
lm[[1, 2]]
lm[, 2]
