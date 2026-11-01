# A lazy-load database stores a promise's environment; forcing the promise
# after lazyLoad() evaluates its code there, not in the global environment.
# R: serialize.c do_lazyLoadDBfetch — eval(val, R_GlobalEnv), where eval of a
# PROMSXP forces it in its own PRENV (eval.c).
invisible(loadNamespace("tools"))

e <- new.env()
assign("f", function() 40, envir = e)
delayedAssign("x", f() + 2, eval.env = e, assign.env = e)

dir <- tempfile("db")
dir.create(dir)
dbbase <- file.path(dir, "test")
tools:::makeLazyLoadDB(e, dbbase)
rm(list = ls(e, all.names = TRUE), envir = e)
cat("before:", exists("x", envir = e, inherits = FALSE), "\n")

lazyLoad(dbbase, envir = e)
print(e$x)
print(e$f())
unlink(dir, recursive = TRUE)
cat("ok\n")
