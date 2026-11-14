# a setter's error: dimnames-type
x <- 1:4; dim(x) <- c(2L, 2L); dimnames(x) <- list(sum, NULL)
