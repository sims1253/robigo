# `s` into a FUN that lapply would run inline forces only FUN's first
# argument before entry (R_forceAndCall(call, 1, rho)), so `...` sees a
# value assigned at the step prompt.
inner <- function(x, y) x + y
for (i in 1:10) inner(i, 2)
g <- 2
ol <- function(X, FUN, ...) { browser(); .Internal(lapply(X, FUN)) }
ol(1, inner, g)
s
s
g <<- 40
c
cat("end\n")
