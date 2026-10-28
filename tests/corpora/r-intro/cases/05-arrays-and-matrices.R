## An Introduction to R (GNU R 4.5.0), chapter: Arrays and matrices
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)

## ---- Index matrices
x <- array(1:20, dim=c(4,5))   # Generate a 4 by 5 array.
x
i <- array(c(1:3,3:1), dim=c(3,2))
i                             # i is a 3 by 2 index array.
x[i]                          # Extract those elements
x[i] <- 0                     # Replace those elements by zeros.
x


## ---- The array() function
Z <- array(0, c(3,4,2))


## ---- The outer product of two arrays
d <- outer(0:9, 0:9)
fr <- table(outer(d, d, "-"))
plot(fr, xlab="Determinant", ylab="Frequency")


## ---- Matrix facilities
absdet <- function(M) prod(svd(M)$d)
