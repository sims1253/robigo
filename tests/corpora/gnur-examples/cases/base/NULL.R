RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: NULL
### Title: The Null Object
### Aliases: NULL as.null as.null.default is.null
### Keywords: attribute manip list sysdata

### ** Examples

is.null(list())     # FALSE (on purpose!)
is.null(pairlist()) # TRUE
is.null(integer(0)) # FALSE
is.null(logical(0)) # FALSE
as.null(list(a = 1, b = "c"))



