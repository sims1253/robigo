# Logical-mask subsetting, matrix row/column subsetting, and recycling arithmetic
x <- c(a = 1, b = 2, c = 3, d = 4); x[c(TRUE, FALSE, TRUE, FALSE)]; x[x > 2]; x[c(TRUE, NA, FALSE, TRUE)]; x[TRUE]; x[c(TRUE, FALSE)]
y <- 1:10; attr(y, "foo") <- "bar"; y[y %% 2 == 0]
m <- matrix(1:6, 2); m[m > 2]; m[c(TRUE, FALSE, TRUE, FALSE, TRUE, FALSE)]
a <- array(1:3, 3, dimnames = list(c("p", "q", "r"))); a[c(TRUE, FALSE, TRUE)]
l <- list(a = 1, b = "x", c = NULL); l[c(TRUE, TRUE, FALSE)]
s <- c("u", NA, "w"); s[!is.na(s)]
f <- factor(c("u", "v", "u")); f[c(TRUE, FALSE, TRUE)]
z <- c(1+2i, 3i); z[c(FALSE, TRUE)]; r <- as.raw(1:3); r[c(TRUE, FALSE, TRUE)]
x[logical(4)]; x[c(TRUE, TRUE, TRUE, TRUE, TRUE)]
e <- expression(a, b, c); e[c(TRUE, FALSE, TRUE)]
g <- function(v) v[v > mean(v)]; g(c(5, 1, 9, 3))
m <- matrix(1:12, 3, dimnames = list(c("a", "b", "c"), NULL)); m[2:3, ]; m[c(TRUE, FALSE, TRUE), 2, drop = FALSE]; m[, 4:1]; m[NA, 1]; m[c(1, NA), 2:3]
tryCatch(m[4, 1], error = function(e) conditionMessage(e)); tryCatch(m[1, 5], error = function(e) conditionMessage(e)); m[integer(0), 7 - 6]; tryCatch(m[4, integer(0)], error = function(e) conditionMessage(e))
l <- matrix(list(1, "a", TRUE, NULL), 2); l[2, ]; s <- matrix(letters[1:6], 2); s[, -1]
x <- matrix(as.numeric(1:20000), 4000); g <- rep(c(TRUE, FALSE), 2000); dim(x[g, , drop = FALSE]); sum(x[g, 2:4])
m <- matrix(1:12, 4); m * 1:4; m + c(0.5, 1.5); 1:6 / 1:3; (1:6)^c(1, 2); c(1.5, 2) - matrix(1:4, 2); 1:6 %% 1:3; 1:6 %/% c(4L, 5L); 7:12 > 1:2; (1:6 == 1:3) & c(TRUE, FALSE)
tryCatch(1:5 + 1:2, warning = function(w) conditionMessage(w)); (1:5 * 1:2)
c(TRUE, NA) | rep(c(FALSE, TRUE, NA), 2)
