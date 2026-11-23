# INTERIM(C1), until V4 step 3 (recorded, not ratcheted; #964): R code
# run inside a compile sees no frame of it in robigo-rt, whose compiler is
# Rust (conditions.rs, try_cmpfun), where GNU R's sees tryCmpfun's
# tryCatch frames and cmp.R's. A calling handler's sys.function(-1) and
# parent.frame() agree (its caller is the warning's restart code in both);
# the traceback of its stop(), which leaves the call, lacks cmp.R's calls.
# Run under LC_ALL=C.
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
h <- function(w) {
  print(names(formals(sys.function(-1L))))
  print(identical(parent.frame(), globalenv()))
  stop("from the handler")
}
withCallingHandlers(f(), warning = h)
