# a setter's error: tsp-empty
x <- .Internal(vector("integer", 0L)); attr(x, "tsp") <- c(1, 1, 1)
