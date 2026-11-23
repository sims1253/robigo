x <- c(a = 1, b = 2, c = 3)
x[["b"]]
x[c("a", "c")]
x[-2]
x[0]
x[c(-1, -3)]
l <- list(a = 1, b = "two", c = list(d = 4))
l$a
l[["b"]]
l$c$d
l[["c"]][["d"]]
l["a"]
l[c("a", "b")]
names(l)
l$zz
m <- matrix(1:6, nrow = 2)
m
m[2, 3]
m[1, ]
m[, 2]
m[, 2, drop = FALSE]
dim(m)
t(m)
nested <- list(1, list(2, list(3)))
nested[[2]][[2]][[1]]
nested[[c(2, 2, 1)]]
v <- 1:10
v[v > 5 & v < 9]
v[c(-1, -2)]
letters[1:3]
letters[25:26]
letters[c(TRUE, FALSE, FALSE)]
