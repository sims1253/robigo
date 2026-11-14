# A promise compiled code makes runs code compiled with it (cmp.R's
# cmpCallArgs: genCode before MAKEPROM), with the decisions of that
# compile, whatever is bound when the promise is forced.
change <- FALSE
g <- function(x) {
  if (change) { e <- globalenv(); e[["+"]] <- function(a, b) 99 }
  x
}
f <- function() g(1 + 1)
print(f())
change <- TRUE
print(f())
print(f())
print(1 + 1)
h <- function() g(2 + 2)
print(h())
print(h())
