# GNU R's compiler keeps one constant for 0 and -0 in a function (its
# constant pool compares with identical()), so the -0 of a compiled function
# that also has a 0 is 0.
f <- function() { z <- 0; w <- -0; c(1 / z, 1 / w, 1 / max(-0, z)) }
print(f()); print(f()); print(f())
g <- function() 1 / -0
print(g()); print(g())
