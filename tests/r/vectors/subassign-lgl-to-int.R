# Assigning an integer into a logical vector makes it integer, attributes
# kept; other references to the logical vector keep it.
x <- c(a = TRUE, b = NA, c = FALSE); attr(x, "foo") <- "bar"
y <- x
x[[2]] <- 5L
print(x); print(y); print(attributes(x))
z <- logical(3); z[2] <- 7L; print(z); print(typeof(z))
m <- matrix(c(TRUE, FALSE, NA, TRUE), 2); m[1, 2] <- 3L; print(m)
f <- function(n) { v <- vector(length = n); for (i in 1:n) v[[i]] <- i; v }
for (k in 1:3) print(f(30))
g <- function() { a <- c(TRUE, NA); b <- a; a[1] <- 2L; list(a, b) }
for (k in 1:3) print(g())
h <- function() { b <- (a <- c(TRUE, FALSE)); a[[2]] <- 9L; list(a, b) }
for (k in 1:3) print(h())
k <- function() { a <- c(TRUE, FALSE); l <- list(a); a[[1]] <- 4L; list(a, l) }
print(k())
q <- c(TRUE, NA); q2 <- q; q[1] <- 1L; print(identical(q2, c(TRUE, NA))); print(q)
