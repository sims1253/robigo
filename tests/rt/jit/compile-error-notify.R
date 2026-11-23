# tryCmpfun (#964): with compilerOptions$suppressAll FALSE, the
# compiler's message for a failed compile, then the closure runs
# uncompiled. Run under LC_ALL=C.
old <- compiler:::setCompilerOptions(suppressAll = FALSE)
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
options(warn = 2)
r <- try(f(), silent = TRUE); cat(r)
