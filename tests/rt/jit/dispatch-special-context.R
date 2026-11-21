# DispatchOrEval for the specials `$` and `[` (eval.c): the method runs in
# a context of its own, so sys.call, sys.function, sys.nframe and
# parent.frame see it, on.exit in the method runs when it returns or
# fails, and an error in the method reports the method's call. Each runs
# from byte code (a loop compiles f) and from the AST.
`$.ctx` <- function(x, name) {
    on.exit(log <<- c(log, paste("exit", name)))
    if (name == "boom") stop("bad member: ", name)
    list(call = sys.call(), fn = deparse(sys.function())[1L],
         n = sys.nframe(), pf = identical(parent.frame(), environment(f)),
         calls = length(sys.calls()))
}
`[.ctx` <- function(x, i) {
    on.exit(log <<- c(log, paste("exit [", i)))
    if (i > 10) stop("index too big")
    list(call = sys.call(), n = sys.nframe(), i = i)
}
x <- structure(list(), class = "ctx")
log <- character()
f <- function() list(x$a, x[2], local(x$b))
for (k in 1:3) r <- f()
print(r)
print(log)
log <- character()
r <- eval(quote(list(x$a, x[3])))
print(r)
print(log)
# errors: the call is the method's, on.exit still runs
log <- character()
g <- function() x$boom
for (k in 1:2) print(tryCatch(g(), error = function(e) conditionCall(e)))
h <- function() x[11]
for (k in 1:2) print(tryCatch(h(), error = function(e) conditionMessage(e)))
print(log)
# the frames during the method: the caller, then the method
`$.frames` <- function(x, name) vapply(sys.calls(), function(cl) deparse(cl)[1L], "")
y <- structure(list(), class = "frames")
k <- function() y$q
for (i in 1:2) print(k())
