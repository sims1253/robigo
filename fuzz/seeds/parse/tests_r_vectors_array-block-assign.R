# x[, , k] <- y: whole leading dimensions, scalar trailing subscripts.
x <- array(0, c(3, 2, 4))
x[, , 2] <- as.double(1:6)
x[, 1, 3] <- c(7, 8, 9)
x[, , 4.9] <- -(1:6) + 0.5
print(x)
y <- x
y[, , 1] <- 100 + 1:6
print(identical(x[, , 1], matrix(0, 3, 2)))
print(y[, , 1])
i <- array(0L, c(2, 2, 2))
i[, , 2] <- 1:4
print(i)
# Other types and lengths take the general path.
i[, , 1] <- c(1.5, 2.5, 3.5, 4.5)
print(i)
x[, , 1] <- 1:6
print(typeof(x)); print(x[, , 1])
x[, , 3] <- 1
print(x[, , 3])
x[, , 2] <- c(TRUE, FALSE)
print(x[, , 2])
z <- array(0i, c(2, 2, 2)); z[, , 1] <- 1:4 + 1i; print(z)
r <- tryCatch({ x[, , 5] <- 1:6; "no error" }, error = function(e) conditionMessage(e)); print(r)
r <- tryCatch({ x[, , 0] <- 1:6; x }, error = function(e) conditionMessage(e)); print(dim(r))
r <- tryCatch({ x[, , 1] <- 1:5; "no error" }, error = function(e) conditionMessage(e)); print(r)
r <- tryCatch({ x[, , NA] <- 1:6; "no error" }, error = function(e) conditionMessage(e)); print(r)
a <- array(1:24 + 0, 2:4, dimnames = list(c("a", "b"), NULL, NULL))
a[, , 2] <- 0
a[, , 3] <- a[, , 1] * 10
print(a)
f <- function() { X <- array(0, c(2, 2, 3)); for (k in 1:3) X[, , k] <- k * c(1, 2, 3, 4); X }
print(f()); print(f())
