# Same as calls-promise-wrapper.R through force(): the wrapper closure's
# frame appears between force() and the erroring primitive (issue #157).

g <- function(x) force(x)
g(identical(1$foo, 2))
