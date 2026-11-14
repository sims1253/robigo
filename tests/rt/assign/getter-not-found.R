`third<-` <- function(x, value) {
  x[3] <- value
  x
}
z <- 1:4
third(z)[1] <- 0L
