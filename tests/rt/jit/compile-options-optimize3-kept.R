# INTERIM(C1) (recorded, not ratcheted; ruled with optimize3, owned by C1):
# at optimize 3 GNU R keeps the code, whose unguarded inline of `abs` does
# not see the attached `abs` at the second call; robigo-rt declines level
# 3's code and the closure runs uncompiled.
invisible(compiler::enableJIT(2))
invisible(compiler::setCompilerOptions(optimize = 3))
attach(list(abs = function(x) 99), name = "package:base", warn.conflicts = FALSE)
f <- function(x) abs(x)
print(f(-7))
print(f(-7))
