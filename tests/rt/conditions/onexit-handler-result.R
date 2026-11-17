# An error caught by an exiting handler unwinds through functions with
# on.exit code: each record lets go of its code, then the handler's result
# is fixed up (R_FixupExitingHandlerResult, which allocates), then the code
# runs (GNU R's endcontext PROTECTs it across the fixup). parse() of a file
# registers on.exit(close(file)) and raises a syntax error, as
# tests/rt/parse/file.R does; under ROBIGO_RETRY_TORTURE=1 (A1's E9
# prototype) the code was freed before it ran.
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e))
f <- function() {
    on.exit(cat("f's exit\n"))
    g()
}
g <- function() {
    on.exit(cat("g's exit\n"), add = TRUE)
    on.exit(cat("g's second exit\n"), add = TRUE)
    stop("boom")
}
for (i in 1:3) print(msg(f()))
r <- tryCatch(f(), error = function(e) e)
print(class(r))
print(conditionMessage(r))
path <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-onexit-", Sys.getpid(), ".R")
writeLines(c("a <- 1", "b c"), path)
for (i in 1:3) print(inherits(tryCatch(parse(path), error = function(e) e), "error"))
print(file.remove(path))
