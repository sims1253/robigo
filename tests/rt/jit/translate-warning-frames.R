# INTERIM(C1), recorded, not ratcheted: cmp.R's compile-time warnings are
# raised by robigo-rt's compiler, with no cmp.R frames on the stack, so a
# handler's sys.calls() lacks cmpfun's, findLocals' and addVar's calls (and
# an error a handler raises has no cmp.R calls in its traceback). V4 step 3
# (cmp.R's own code compiling) closes it. Run under LC_ALL=C.
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
n <- 0
withCallingHandlers(f(), warning = function(w) {
  n <<- n + 1
  if (n == 1) print(vapply(sys.calls(), function(x) deparse(x[[1]])[1], ""))
  invokeRestart("muffleWarning")
})
