# .Random.seed validation (GetRNGkind), restore and kind switching
.Random.seed <- 1:3
.Internal(runif(1, 0, 1)) > 2
exists(".Random.seed")
length(.Random.seed)
.Random.seed <- c(1.5, 2)
x <- .Internal(runif(1, 0, 1))
length(.Random.seed)
.Random.seed <- -5L
x <- .Internal(runif(1, 0, 1))
.Random.seed <- 10403L
x <- .Internal(runif(1, 0, 1))
length(.Random.seed)
.Random.seed <- 10405L
x <- .Internal(runif(1, 0, 1))
.Random.seed <- 10903L
x <- .Internal(runif(1, 0, 1))
.Random.seed <- 99L
x <- .Internal(runif(1, 0, 1))
.Random.seed <- "a"
x <- .Internal(runif(1, 0, 1))
set.seed(3)
s <- .Random.seed
a <- .Internal(rnorm(3, 0, 1))
.Random.seed <- s
b <- .Internal(rnorm(3, 0, 1))
identical(a, b)

.Internal(set.seed(10L, 3L, 4L, 1L))
.Internal(runif(2, 0, 1))
f <- function() { set.seed(4); .Internal(runif(1, 0, 1)) }
f()
.Internal(runif(1, 0, 1))
.Internal(set.seed(1L, 1L, NULL, NULL))
.Internal(set.seed(1L, 3L, 2L, NULL))
print(.Internal(rnorm(5, 0, 1)), digits = 15)
print(.Internal(rnorm(1, 0, 1)), digits = 15)
.Internal(set.seed(1L, 3L, 4L, NULL))
.Internal(set.seed(NA_integer_, NULL, NULL, NULL))
