# math2 .Internals report like error(): against the calling closure
choose(5, 2)
f <- function(n) choose(n, "b")
f(1)
