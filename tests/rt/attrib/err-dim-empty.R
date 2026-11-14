# a setter's error: dim-empty
x <- 1:6; dim(x) <- .Internal(vector("integer", 0L))
