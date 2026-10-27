RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Comparison
### Title: Relational Operators
### Aliases: < <= == != >= > Comparison collation
### Keywords: logic

### ** Examples

x <- stats::rnorm(20)
x < 1
x[x > 0]

x1 <- 0.5 - 0.3
x2 <- 0.3 - 0.1
x1 == x2                   # FALSE on most machines
isTRUE(all.equal(x1, x2))  # TRUE everywhere

## No test: 
##D # range of most 8-bit charsets, as well as of Latin-1 in Unicode
##D z <- c(32:126, 160:255)
##D x <- if(l10n_info()$MBCS) {
##D     intToUtf8(z, multiple = TRUE)
##D } else rawToChar(as.raw(z), multiple = TRUE)
##D ## by number
##D writeLines(strwrap(paste(x, collapse=" "), width = 60))
##D ## by locale collation
##D writeLines(strwrap(paste(sort(x), collapse=" "), width = 60))
## End(No test)


