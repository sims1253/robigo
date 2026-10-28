## An Introduction to R (GNU R 4.5.0), chapter: Introduction and preliminaries
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)

## ---- Data permanency and removing objects
objects()

rm(x, y, z, ink, junk, temp, foo, bar)
