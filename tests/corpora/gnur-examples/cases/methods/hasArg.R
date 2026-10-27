RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: hasArg
### Title: Look for an Argument in the Call
### Aliases: hasArg
### Keywords: programming

### ** Examples

ftest <- function(x1, ...) c(hasArg(x1), hasArg("y2"))

ftest(1) ## c(TRUE, FALSE)
ftest(1, 2)  ## c(TRUE, FALSE)
ftest(y2 = 2)   ## c(FALSE, TRUE)
ftest(y = 2)    ## c(FALSE, FALSE) (no partial matching)
ftest(y2 = 2, x = 1)  ## c(TRUE, TRUE) partial match x1



