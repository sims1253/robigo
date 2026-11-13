# a vector grown one element at a time, at the top level and in a closure
v <- 0:-1
for (i in 1:20) v[i] <- i * i
v
grow <- function(n) {
  w <- 1:0
  for (i in 1:n) w[[i]] <- i %% 3L
  w
}
grow(12)
