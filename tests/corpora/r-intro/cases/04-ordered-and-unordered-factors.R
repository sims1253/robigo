## An Introduction to R (GNU R 4.5.0), chapter: Ordered and unordered factors
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)

## ---- A specific example
state <- c("tas", "sa",  "qld", "nsw", "nsw", "nt",  "wa",  "wa",
             "qld", "vic", "nsw", "vic", "qld", "qld", "sa",  "tas",
             "sa",  "nt",  "wa",  "vic", "qld", "nsw", "nsw", "wa",
             "sa",  "act", "nsw", "vic", "vic", "act")

statef <- factor(state)

statef

levels(statef)


## ---- The function tapply() and ragged arrays
incomes <- c(60, 49, 40, 61, 64, 60, 59, 54, 62, 69, 70, 42, 56,
               61, 61, 61, 58, 51, 48, 65, 49, 49, 41, 48, 52, 46,
               59, 46, 58, 43)

incmeans <- tapply(incomes, statef, mean)

stdError <- function(x) sqrt(var(x)/length(x))

incster <- tapply(incomes, statef, stdError)

incster
