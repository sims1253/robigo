# load and loadFromConn2 (saveload.c's do_loadFromConn2, RestoreToEnv) of
# files GNU R saved (data/README.md): formats 2 and 3, ASCII and
# uncompressed, into an environment, the old-style S4 warning, and the
# errors.
d <- if (dir.exists("data")) "data" else "tests/rt/serialize/data"
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e),
                               warning = function(w) paste("warning:", conditionMessage(w)))
for (file in c("xyf-v2.rda", "xyf-v3.rda", "xy-ascii.rda", "xy-plain.rda")) {
    e <- new.env()
    print(load(file.path(d, file), envir = e))
    print(ls(e, sorted = TRUE))
    print(e$x)
    if (exists("y", envir = e)) print(e$y)
    if (exists("f", envir = e)) { print(formals(e$f)); print(body(e$f)); print(identical(environment(e$f), globalenv())) }
}
e <- new.env()
print(load(file.path(d, "none.rda"), envir = e))
print(ls(e))
# the result is invisible, the bindings replace
x <- "old"
withVisible(load(file.path(d, "xy-plain.rda")))$visible
print(x)
print(msg(load(file.path(d, "old-s4.rda"), envir = new.env())))
r <- withCallingHandlers(load(file.path(d, "old-s4.rda"), envir = e),
                         warning = function(w) { print(conditionMessage(w)); print(conditionCall(w)); invokeRestart("muffleWarning") })
print(r)
print(class(e$old))
# a connection load opens, or one already open
con <- gzfile(file.path(d, "xyf-v3.rda"))
print(load(con, envir = new.env()))
print(isOpen(con))
close(con)
con <- gzfile(file.path(d, "xyf-v3.rda"), "rb")
print(load(con, envir = new.env()))
print(isOpen(con))
close(con)
# the errors
con <- file(file.path(d, "xy-plain.rda"))
print(msg(.Internal(loadFromConn2(con, NULL, FALSE))))
print(msg(.Internal(loadFromConn2(con, list(), FALSE))))
close(con)
con <- file(file.path(d, "xy-plain.rda"), "r")
print(msg(load(con)))
print(msg(withCallingHandlers(load(con), warning = function(w) invokeRestart("muffleWarning"))))
print(msg(.Internal(loadFromConn2(con, new.env(), FALSE))))
close(con)
con <- file(file.path(d, "README.md"))
print(msg(.Internal(loadFromConn2(con, new.env(), FALSE))))
close(con)
f <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-load-", Sys.getpid())
writeLines(character(0), f)
con <- file(f)
print(msg(.Internal(loadFromConn2(con, new.env(), FALSE))))
close(con)
print(msg(load(f)))
con <- file(f, "w")
print(msg(.Internal(loadFromConn2(con, new.env(), FALSE))))
close(con)
invisible(file.remove(f))
print(msg(load(42)))
