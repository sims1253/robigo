# The link-walk family (link-walk.R): lapply names its answer by X's names
# before the loop. FUN's write to y (only its binding and lapply's C local
# hold it) is made in place, and the answer keeps the names y had.
y <- c(p = 1, q = 2)
f <- function(e) { names(y)[1] <<- "zz"; e }
g <- function(...) .Internal(lapply(y, f))
print(names(g()))
print(names(y))
y <- c(p = 1, q = 2)
f2 <- function(e) { attr(y, "names") <<- c("u", "v"); e }
g2 <- function(...) .Internal(lapply(y, f2))
print(names(g2()))
print(names(y))
y <- c(p = 1, q = 2)
print(names(g2()))
y <- c(p = 1, q = 2)
print(names(lapply(y, f2)))
