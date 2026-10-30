x <- 1:5
x[x[4] <- 2] <- (x[4] <- 100)
print(x)
x <- 1:5
tryCatch(x[{x <- 9L; c(NA, 1L)}] <- 1:2,
         error = function(e) cat(conditionMessage(e), "\n"))
print(x)
x <- 1:5
tryCatch(x[{rm(x); c(NA, 1L)}] <- 1:2,
         error = function(e) cat(conditionMessage(e), "\n"))
print(exists("x"))
f <- function(x) { x[{x <- 9L; 2L}] <- 100L; x }
print(f(1:5))
