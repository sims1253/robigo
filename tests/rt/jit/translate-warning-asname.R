# INTERIM(C1), recorded, not ratcheted: cmpAssign's warning for an
# assignment to a string that does not translate (`as.name(getAssignedVar(e,
# cntxt))`) comes in the middle of GNU R's compile, so a handler of it can
# change what the rest of the compile decides (here a global `+`). robigo-rt
# raises it after the compile. `<<-` makes no local, so findLocals gives no
# warning of its own. V4 step 3 (cmp.R's own code compiling) closes it. Run
# under LC_ALL=C.
f <- function(x) { "\u4e2d" <<- 1; for (i in 1) NULL; x + 1 }
withCallingHandlers(print(f(1)), warning = function(w) {
  assign("+", function(e1, e2) 99, envir = globalenv())
  invokeRestart("muffleWarning")
})
print(f(1))
