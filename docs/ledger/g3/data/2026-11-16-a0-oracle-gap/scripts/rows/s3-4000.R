s <- 0
area <- function(s) UseMethod("area")
area.sq <- function(s) s$side^2
area.default <- function(s) 0
sq <- structure(list(side = 2), class = "sq")

g <- function() { for (i in 1:4000) { s <- s + area(sq) }; NULL }
g()
