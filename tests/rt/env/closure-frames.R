# Closure frames list newest first with the arguments last in formal order;
# rm then reinsertion moves a binding to the front; dynamic definitions
# (assign, a loop variable, delayedAssign, makeActiveBinding) interleave by
# definition time; unbound locals are invisible (docs/g3.md §5.1).
f <- function(a, b) {
  z <- 1; y <- 2
  print(ls(sorted = FALSE))
  rm(z); z <- 3
  print(ls(sorted = FALSE))
  y <- 4
  print(ls(sorted = FALSE))
  print(exists("w", inherits = FALSE))
  print(missing(b))
  print(names(as.list(environment(), all.names = TRUE)))
  if (FALSE) w <- 0
  print(exists("w", inherits = FALSE))
}
f(1)
g <- function(a) {
  assign("q", 1); x <- 1; assign("r", 2)
  print(ls(sorted = FALSE))
  for (n in c("o1", "o2", "o3", "o4", "o5", "o6")) assign(n, 1)
  print(ls(sorted = FALSE))
  rm(q, o3); assign("q", 9); x <- 5
  print(ls(sorted = FALSE))
  print(mget(c("x", "q", "a")))
  print(get("q"))
  print(exists("o3", inherits = FALSE))
}
g(7)
h <- function(a, b = 2, ...) {
  print(ls(sorted = FALSE, all.names = TRUE))
  print(b)
  print(ls(sorted = FALSE, all.names = TRUE))
}
h(1)
h(1, c = 3)
k <- function() {
  delayedAssign("lazy", stop("never"))
  makeActiveBinding("act", function() 1, environment())
  v <- 1
  print(ls(sorted = FALSE))
  print(act)
}
k()
