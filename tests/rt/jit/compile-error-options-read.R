# INTERIM, until V4 step 3 (recorded, not ratcheted; #964): GNU R's cmp.R
# reads compilerOptions in every compile (make.toplevelContext's
# getCompilerOption("suppressAll"), cmp.R:1003, and the other options),
# robigo-rt's Rust compiler reads none of them; only tryCmpfun's handler
# reads suppressAll (conditions.rs, notify_compiler_error). An active
# binding shows the reads: GNU R's two for a failed compile, robigo-rt's
# one. Run under LC_ALL=C.
options(warn = 2)
o <- compiler:::compilerOptions
rm("suppressAll", envir = o)
makeActiveBinding("suppressAll", function() { cat("active read\n"); FALSE }, o)
f <- function() { "\u4e2d" <- 1L; for (i in 1) NULL; 0 }
r <- try(f(), silent = TRUE); cat(r)
