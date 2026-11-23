# GNU R's cmp.R reads compilerOptions in every compile: its top-level
# context reads optimize, suppressAll, suppressNoSuperAssignVar and
# suppressUndefined (make.toplevelContext, cmp.R:1002), and each function
# context again after its funEnv. robigo-rt's compile reads them at the same
# points, as do tryCmpfun's handler's reads (#964). An active binding shows
# the reads: for this failed compile, the entry's and the handler's (funEnv
# fails before the function context reads). Run under LC_ALL=C.
options(warn = 2)
o <- compiler:::compilerOptions
rm("suppressAll", envir = o)
makeActiveBinding("suppressAll", function() { cat("active read\n"); FALSE }, o)
f <- function() { "\u4e2d" <- 1L; for (i in 1) NULL; 0 }
r <- try(f(), silent = TRUE); cat(r)
