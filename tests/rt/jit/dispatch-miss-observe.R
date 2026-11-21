# What could see DispatchOrEval's setup when no method is found (eval.c):
# none of it runs code, and what runs afterwards (the default's arguments,
# an active binding the default reads) runs after its endcontext. Each from
# byte code (loops compile the functions) and from the AST.
x <- structure(list(a = 1, b = 2), class = "k")
# the default's arguments are evaluated after the dispatch's context: the
# frames, the call and the function they see are the caller's
at <- function() list(n = sys.nframe(), calls = vapply(sys.calls(), function(c) deparse(c)[1L], ""),
                      fn = deparse(sys.function(-1L))[1L], caller = deparse(sys.call(-1L)))
s <- function() x[{ w <<- at(); 2 }]
for (i in 1:3) { r <- s(); print(w) }
print(r)
eval(quote(x[{ w <<- at(); 1 }])); print(w)
# on.exit of the caller runs once, after the default
log <- character()
o <- function() { on.exit(log <<- c(log, "exit o")); log <<- c(log, "before"); v <- x$a; log <<- c(log, "after"); v }
for (i in 1:2) print(o())
print(log)
# a method defined between two calls of the same compiled function
m <- function() list(x$a, x[1], length(x))
for (i in 1:3) print(m())
`$.k` <- function(x, name) "now a method"
print(m())
rm(`$.k`)
print(m())
# the object an active binding: read once per `$`, before dispatch (from
# the AST only: GNU R's byte code reads an active binding twice per GETVAR
# once compiled, with or without a class, which robigo-rt does not yet)
reads <- 0
makeActiveBinding("ab", function() { reads <<- reads + 1; x }, environment())
print(ab$a)
print(ab$b)
print(ab[1])
print(reads)
# the object a promise: forced once, before dispatch
pf <- function(obj) list(obj$a, obj$b)
for (i in 1:2) print(pf({ cat("forcing\n"); x }))
# a classed environment: the default reads its active binding after the
# dispatch's context, so the binding's function sees the caller's frames
e <- structure(new.env(), class = "ke")
makeActiveBinding("v", function() sys.nframe(), e)
ef <- function() e$v
for (i in 1:3) print(ef())
print(e$v)
# `...` in the call takes the whole way: an error when there is none
dd <- function() x[...]
for (i in 1:2) print(tryCatch(dd(), error = function(c) conditionMessage(c)))
