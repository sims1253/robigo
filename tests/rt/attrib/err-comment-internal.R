# a setter's error: comment-internal
x <- 1; .Internal("comment<-"(x, 2))
