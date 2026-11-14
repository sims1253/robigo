# a setter's error: unclass-env
e <- new.env(); class(e) <- "foo"; unclass(e)
