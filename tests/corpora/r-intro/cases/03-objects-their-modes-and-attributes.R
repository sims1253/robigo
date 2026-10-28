## An Introduction to R (GNU R 4.5.0), chapter: Objects, their modes and attributes
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)

## ---- Intrinsic attributes: mode and length
z <- 0:9

digits <- as.character(z)

d <- as.integer(digits)


## ---- Changing the length of an object
e <- numeric()

e[3] <- 17
