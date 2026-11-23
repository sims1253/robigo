f <- function() 1
debug(f); environment(f) <- new.env(); isdebugged(f)
g <- function() 1
debug(g); h <- g; environment(h) <- new.env(); isdebugged(h); isdebugged(g)
