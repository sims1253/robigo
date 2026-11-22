# INTERIM(C1), recorded, not ratcheted (it predates the translate-warning
# PR): under byte code, an error in a direct replacement call such as
# `dim<-`(1, value = 2L) is reported against R_getBCInterpreterExpression's
# inflation of it, dim(1) <- 2L (eval.c:inflateAssignmentCall). robigo-rt's
# Cx::bc_interpreter_expression does not make the inflated call (under &Cx
# on an error's path) and reports the function's call. A complex
# assignment, which robigo-rt sets inflated itself, is right.
f <- function() { for (i in 1) NULL; `dim<-`(1, value = 2L) }
print(tryCatch(f(), error = function(e) conditionCall(e)))
g <- function() { for (i in 1) NULL; x <- 1; dim(x) <- 2L }
print(tryCatch(g(), error = function(e) conditionCall(e)))
