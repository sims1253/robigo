# Classed vectors: order() and sort() go through xtfrm(); rank() compares
# an object's elements with .gt() (here a `>` that reports each call, so
# the comparisons GNU R's Shell sort makes are visible); is.unsorted() of
# a classed list calls .gtn()
x <- structure(c(3, 1, 2, 1, 5, 4, 1), class = "ver")
`[.ver` <- function(x, i) structure(unclass(x)[i], class = "ver")
`==.ver` <- function(e1, e2) unclass(e1) == unclass(e2)
n <- 0
`>.ver` <- function(e1, e2) { n <<- n + 1; unclass(e1) > unclass(e2) }
print(rank(x))
print(n)
n <- 0
print(rank(x, ties.method = "max"))
print(n)
xtfrm.ver <- function(x) -unclass(x)
print(order(x))
print(sort(x))
print(is.unsorted(x))
l <- structure(list(1, 2, 2), class = "ver")
`[.ver` <- function(x, i) structure(unclass(x)[i], class = "ver")
`>=.ver` <- function(e1, e2) unlist(unclass(e1)) >= unlist(unclass(e2))
`>.ver` <- function(e1, e2) unlist(unclass(e1)) > unlist(unclass(e2))
print(is.unsorted(l))
print(is.unsorted(l, strictly = TRUE))
