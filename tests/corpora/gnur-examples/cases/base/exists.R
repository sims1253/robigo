RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: exists
### Title: Is an Object Defined?
### Aliases: exists get0
### Keywords: data

### ** Examples

##  Define a substitute function if necessary:
if(!exists("some.fun", mode = "function"))
  some.fun <- function(x) { cat("some.fun(x)\n"); x }
search()
exists("ls", 2) # true even though ls is in pos = 3
exists("ls", 2, inherits = FALSE) # false

## These are true (in most circumstances):
identical(ls,   get0("ls"))
identical(NULL, get0(".foo.bar.")) # default ifnotfound = NULL (!)
## Don't show: 
stopifnot(identical(ls, get0("ls")),
          is.null(get0(".foo.bar.")))
## End(Don't show)



