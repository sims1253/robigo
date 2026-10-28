## An Introduction to R (GNU R 4.5.0), chapter: Lists and data frames
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)

## ---- Lists
Lst <- list(name="Fred", wife="Mary", no.children=3,
              child.ages=c(4,7,9))

x <- "name"; Lst[[x]]


## ---- Data frames
detach()

search()

search()
ls(2)
