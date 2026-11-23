# tryCmpfun (#964): a compile an error ends leaves the closure
# uncompiled (NOJIT): not compiled again, so cmp.R's warning is not given
# again. Run under LC_ALL=C.
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
options(warn = 2)
r <- try(f(), silent = TRUE); cat(r)
options(warn = 1)
f()
f()
