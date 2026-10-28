## An Introduction to R (GNU R 4.5.0), chapter: Simple manipulations; numbers and vectors
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)

## ---- Vectors and assignment
x <- c(10.4, 5.6, 3.1, 6.4, 21.7)

assign("x", c(10.4, 5.6, 3.1, 6.4, 21.7))

c(10.4, 5.6, 3.1, 6.4, 21.7) -> x

1/x

y <- c(x, 0, x)


## ---- Vector arithmetic
v <- 2*x + y + 1

sum((x-mean(x))^2)/(length(x)-1)

sqrt(-17)

sqrt(-17+0i)


## ---- Generating regular sequences
seq(-5, 5, by=.2) -> s3

s4 <- seq(length=51, from=-5, by=.2)

s5 <- rep(x, times=5)

s6 <- rep(x, each=5)


## ---- Logical vectors
temp <- x > 13


## ---- Missing values
z <- c(1:3,NA);  ind <- is.na(z)

0/0

Inf - Inf


## ---- Character vectors
labs <- paste(c("X","Y"), 1:10, sep="")

c("X1", "Y2", "X3", "Y4", "X5", "Y6", "X7", "Y8", "X9", "Y10")


## ---- Index vectors; selecting and modifying subsets of a data set
y <- x[!is.na(x)]

(x+1)[(!is.na(x)) & x>0] -> z

x[1:10]

c("x","y")[rep(c(1,2,2,1), times=4)]

y <- x[-(1:5)]

fruit <- c(5, 10, 1, 20)
names(fruit) <- c("orange", "banana", "apple", "peach")
lunch <- fruit[c("apple","orange")]

x[is.na(x)] <- 0

y[y < 0] <- -y[y < 0]

y <- abs(y)
