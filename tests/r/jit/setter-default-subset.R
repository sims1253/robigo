# [<- also retains the sentinel for classed not-subsettable errors.
f <- function(b, i) { b[i] <- 1 }
f(f, 3)
