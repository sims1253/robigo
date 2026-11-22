# INTERIM(C1), issue #964 (recorded, not ratcheted): under options(warn = 2)
# a compile-time warning of cmp.R's is an error inside the compile, which
# tryCmpfun's handler takes (the compile fails; the closure runs uncompiled
# and raises its own error, against its call). Run under LC_ALL=C.
options(warn = 2)
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; ls() }
g <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
r <- try(g(), silent = TRUE); cat(r)
f()
