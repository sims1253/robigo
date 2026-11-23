# order()/sort() on vectors long enough for the radix passes (n >= 256)
x <- sin(1:1000) * 1e3
o <- order(x)
o[1:10]
o[991:1000]
all(diff(x[o]) >= 0)
od <- order(x, decreasing = TRUE)
identical(od, rev(o))
# ties keep input order (stable)
y <- rep(c(3, 1, 2, NA, -0, 0, Inf, -Inf), 50)
oy <- order(y)
oy[1:20]
oy[381:400]
order(y, na.last = FALSE)[1:12]
length(order(y, na.last = NA))
sort(y)[c(1:3, 349:352)]
sort(y, decreasing = TRUE)[1:5]
# integers, with negatives and NA
z <- as.integer(round(cos(1:600) * 100))
z[c(5, 77)] <- NA
oz <- order(z)
oz[1:10]
oz[595:600]
sort(z)[1:5]
order(z, decreasing = TRUE)[1:8]
# logical
l <- rep(c(TRUE, NA, FALSE), 100)
order(l)[c(1:3, 100:103, 200:203, 298:300)]
# two keys
a <- rep(1:3, length.out = 300)
b <- sin(1:300)
o2 <- order(a, b)
o2[c(1:5, 100:105, 296:300)]
o3 <- order(a, -b, decreasing = c(TRUE, FALSE), method = "radix")
o3[c(1:5, 296:300)]
# grouping via split/unique paths
g <- sin(1:400) > 0
table(g)
as.vector(sapply(split(1:400, g), length))
# rep variants
rep(1:3, length.out = 7)
rep(c(a = 1, b = 2), times = 3)
rep(1:2, each = 3, length.out = 5)
rep(1:3, times = c(2, 0, 3))
rep(1:2, times = c(1, 2, 3, 4), each = 2)
rep(c("x", "y"), each = 2, times = 2)
rep(list(1, "a"), length.out = 3)
rep(1:3, each = 0)
rep(numeric(0), length.out = 3)
# relational ops with recycling and NA
c(1, NA, 3, NaN) > 2
1:6 == c(1L, NA)
c(1, 2, 3) <= 2L
5L != c(5, NA, 6)
c(TRUE, NA) < 1:4
