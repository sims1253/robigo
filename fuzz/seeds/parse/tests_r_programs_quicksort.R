qs <- function(v) {
  if (length(v) <= 1) return(v)
  p <- v[1]
  rest <- v[-1]
  c(qs(rest[rest < p]), p, qs(rest[rest >= p]))
}
qs(c(3, 6, 1, 8, 2, 9, 4, 7, 5, 0))
bubble <- function(v) {
  n <- length(v)
  for (i in 1:(n - 1)) for (j in 1:(n - i)) if (v[j] > v[j + 1]) {
    t <- v[j]; v[j] <- v[j + 1]; v[j + 1] <- t
  }
  v
}
bubble(c(5, 2, 9, 1, 5, 6))
