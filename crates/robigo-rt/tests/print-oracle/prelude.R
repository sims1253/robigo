# GNU R's side of tests/print.rs: the constructors that robigo-rt's test
# defines as builtins (the base subset has no c, structure, pairlist,
# as.raw yet).
c2 <- c
structure2 <- structure
pairlist2 <- pairlist
raw2 <- function(...) as.raw(c(...))
