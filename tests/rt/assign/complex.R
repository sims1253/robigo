# complex assignment through `*tmp*`: replacement functions, `<<-`, and
# the copy a closure's argument gets
`second<-` <- function(x, value) {
  x[2] <- value
  x
}
u <- 1:3
second(u) <- 9L
u
f <- function(x) {
  x[1] <- 100L
  x
}
o <- 1:2
f(o)
o
k <- 1:5
bump <- function() k[2] <<- k[2] + 10L
bump()
k
second <- function(x) x[2]
z <- 1:4
second(z)[1] <- 0L
z
r <- (z[3] <- 7L)
r
z
