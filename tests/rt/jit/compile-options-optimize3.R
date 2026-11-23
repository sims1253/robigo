# INTERIM (recorded, not ratcheted): compilerOptions' optimize = 3 inlines
# base's functions without level 2's guard (cmp.R:getInlineInfo, "global"
# and optimize >= 3), so a sqrt defined after the compile is not seen: GNU R
# calls base's sqrt in the compiled closure. robigo-rt's compile implements
# level 2 only and declines at 3: the closure runs uncompiled and calls the
# shadow. Cured by level 3's unguarded inlining in robigo-rt's compiler.
compiler::setCompilerOptions(optimize = 3)
f <- function(x) { for (i in 1:2) NULL; sqrt(x) }
print(f(16))
sqrt <- function(x) "the shadow"
print(f(16))
