# tryCmpfun (#964): only errors: another condition's exiting
# handler takes a warning of the compile. Run under LC_ALL=C.
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
r <- tryCatch(f(), warning = function(w) conditionMessage(w))
print(r)
g <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
r <- tryCatch(g(), condition = function(c) class(c))
print(r)
