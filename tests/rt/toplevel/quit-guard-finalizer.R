# An error in an exit finalizer quit() runs: reported, the next one runs,
# quit()'s status stands; no abandoned frame runs again.
invisible(reg.finalizer(.BaseNamespaceEnv, function(e) stop("in finalizer"), onexit = TRUE))
invisible(reg.finalizer(globalenv(), function(e) cat("second finalizer\n"), onexit = TRUE))
f <- function() { on.exit(cat("f on.exit\n")); quit("no", 4) }
f()
