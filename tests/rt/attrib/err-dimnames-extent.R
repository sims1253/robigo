# a setter's error: dimnames-extent
x <- 1:4; dim(x) <- c(2L, 2L); dimnames(x) <- list(1:3, NULL)
