# a setter's error: dimnames-length
x <- 1:4; dim(x) <- c(2L, 2L); dimnames(x) <- list(NULL, NULL, NULL)
