# INTERIM(C1) (recorded, not ratcheted; a port, owned by C1's rebuilt PR A
# on PR B, #1029): cmpMultiColon accepts `base::identity` and compiles the
# call through cmpCallSymFun, so cmpCall makes no findFunDef check and a
# lazy binding of `::` is not forced. robigo-rt's `::` handler declines
# into the check and forces it.
invisible(compiler::enableJIT(2))
attach(NULL, name = "package:base", warn.conflicts = FALSE)
e <- as.environment(2)
delayedAssign("::", { cat("forced ::\n"); get("::", baseenv()) }, assign.env = e)
f <- function(x) { if (x) base::identity; 7 }
print(f(FALSE))
print(f(FALSE))
