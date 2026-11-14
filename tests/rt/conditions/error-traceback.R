inner <- function(x) stop("failed with ", x)
middle <- function(x) inner(x + 1)
outer <- function(x) middle(x * 2)
r <- tryCatch(outer(1), error = function(e) "caught")
r
cat(geterrmessage(), "\n")
warning("top-level warning")
outer(5)
