# INTERIM (recorded, not ratcheted): with suppressAll not identical(TRUE)
# cmp.R's notes may print, and robigo-rt's compiler has none yet: its
# compile fails where the code's generation begins, and tryCmpfun's
# handler takes the failure. For this clean code GNU R prints no note and
# runs no handler at all: the difference this INTERIM makes, cured by the
# notes' port (cmp.R:2844-2950), which removes the failure.
# - FALSE (setCompilerOptions): the handler prints "Error: compilation
#   failed - robigo: ..." and the closure runs uncompiled.
# - NA and 1, assigned (setCompilerOptions takes only TRUE and FALSE):
#   as FALSE for the compile (cmp.R's identical(cntxt$suppressAll, TRUE));
#   the handler's own `if (!compilerOptions$suppressAll)` then errors for
#   NA and prints nothing for 1.
# (No top-level loop: one would be compiled whole, its closures with it,
# before the option.)
compiler::setCompilerOptions(suppressAll = FALSE)
f1 <- function(x) { for (i in 1:2) NULL; x * 2 }
print(f1(21))
assign("suppressAll", NA, envir = compiler:::compilerOptions)
f2 <- function(x) { for (i in 1:2) NULL; x * 2 }
print(tryCatch(f2(21), error = function(e) conditionMessage(e)))
assign("suppressAll", 1, envir = compiler:::compilerOptions)
f3 <- function(x) { for (i in 1:2) NULL; x * 2 }
print(f3(21))
