# tryCmpfun (#964): the handler calls compiler's notifyCompilerError as it
# is bound at the failed compile, whatever compilerOptions says: a
# replacement runs. Run under LC_ALL=C.
ns <- environment(compiler:::notifyCompilerError)
unlockBinding("notifyCompilerError", ns)
assign("notifyCompilerError", function(msg) cat("NOTIFY:", msg, "\n"), ns)
options(warn = 2)
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
cat(try(f(), silent = TRUE))
