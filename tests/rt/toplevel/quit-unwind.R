# quit() ends R from inside the call: no on.exit, no finally runs, but
# the exit finalizers do.
invisible(reg.finalizer(.BaseNamespaceEnv, function(e) cat("exit finalizer\n"), onexit = TRUE))
f <- function() { on.exit(cat("on.exit\n")); tryCatch(quit("no", 9), finally = cat("finally\n")) }
f()
