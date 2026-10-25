# sample() errors come from error(), i.e. the calling closure
set.seed(1)
f <- function(k) .Internal(sample(5, k, FALSE, NULL))
f(5)
f(6)
