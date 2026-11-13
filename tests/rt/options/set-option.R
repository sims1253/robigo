old <- .Internal(options(robigo.test = 1))
old
getOption("robigo.test")
.Internal(options("robigo.test"))
.Internal(options(robigo.test = NULL))
getOption("robigo.test")
.Internal(options(width = 9))
