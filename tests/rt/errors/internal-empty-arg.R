# An empty argument of an .Internal's inner call: do_internal evaluates the
# arguments with evalList(args, env, call, 0), `call` the .Internal call, so
# "argument N is empty" names it (names.c), at top level, in a function and
# from compiled code (a loop compiles the function); arguments before the
# empty one are evaluated first
cc <- function(e) list(call = conditionCall(e), msg = conditionMessage(e))
log <- character()
side <- function(v) { log <<- c(log, v); v }
print(tryCatch(.Internal(paste0(list(side("a")), , FALSE)), error = cc))
f <- function() .Internal(paste0(list(side("b")), , FALSE))
for (i in 1:3) print(tryCatch(f(), error = cc))
g <- function(...) .Internal(paste0(list(...), , FALSE))
for (i in 1:3) print(tryCatch(g(side("c")), error = cc))
print(log)
# the empty argument after `...`
h <- function(...) .Internal(paste(list(...), " ", ))
for (i in 1:2) print(tryCatch(h("x"), error = cc))
# a builtin's own empty argument still names the builtin's call
k <- function() c(1, , 2)
for (i in 1:2) print(tryCatch(k(), error = cc))
# `.Internal` reached through another name, or an inner pairlist (do_internal
# takes one): the arguments are still the inner call's
ii <- .Internal
fa <- function(...) ii(paste0(...))
for (i in 1:2) print(fa(list("a"), NULL, FALSE))
print(tryCatch(ii(paste0(list("a"), , FALSE)), error = cc))
print(tryCatch((.Internal)(paste0(list("a"), , FALSE)), error = cc))
x <- as.call(list(as.name(".Internal"), as.pairlist(as.list(quote(paste0(...))))))
fp <- function(...) eval(x)
for (i in 1:2) print(fp(list("b"), NULL, FALSE))
# an .Internal builtin as an R value (unserialize makes one): an ordinary
# call of it, not an .Internal wrapper
p <- unserialize(charToRaw(sub("3\nsum\n", "6\npaste0\n",
    rawToChar(serialize(.Primitive("sum"), NULL, ascii = TRUE)), fixed = TRUE)))
pf <- function() p(list("a"), NULL, FALSE)
for (i in 1:2) print(pf())
print(forceAndCall(0, p, list("a"), NULL, FALSE))
# the compiler's call constant for such an .Internal never shows: the
# builtin's own errors and warnings name the inner call, a calling
# handler's sys.calls() and conditionCall see the closure's frames and the
# inner call, and the closure's sys.call() and match.call() are its own
pc <- function(...) .Internal(paste0(list(...), 1, FALSE))
for (i in 1:2) print(tryCatch(pc("a"), error = cc))
fr <- function(...) withCallingHandlers(pc(...), error = function(e) {
    print(vapply(sys.calls(), function(cl) deparse(cl)[1L], ""))
    print(conditionCall(e))
})
for (i in 1:2) try(fr("b"), silent = TRUE)
mm <- function(x, ...) list(sys.call(), match.call(), .Internal(paste0(list(...), NULL, FALSE)))
for (i in 1:2) print(mm(1, "c", "d"))
ww <- function(...) .Internal(as.vector(c(...), "integer"))
for (i in 1:2) print(withCallingHandlers(ww("10", "x"), warning = function(w) { print(conditionCall(w)); invokeRestart("muffleWarning") }))
# an earlier argument that runs compiled code (a compiled closure, a
# promise forced through `...`, a closure that itself calls .Internal, a
# builtin evaluated directly, a caught error) does not change the call
# reported; an empty argument through `...` is the closure's ("argument is
# missing"), as GNU R's evalList forces the dots element
hc <- function() length(1:3)
for (i in 1:2) hc()
kc <- function() .Internal(paste0(list("z"), NULL, FALSE))
xx <- 1:3
p1 <- function() .Internal(paste0(list(hc()), , FALSE))
p2 <- function(...) .Internal(paste0(list(...), , FALSE))
p3 <- function() .Internal(paste0(list(kc()), , FALSE))
p4 <- function() .Internal(paste0(length(xx), , FALSE))
p5 <- function() .Internal(paste0((function() length(xx))(), , FALSE))
p6 <- function(...) .Internal(paste0(sum(...), , FALSE))
p7 <- function() .Internal(paste0(tryCatch(stop("u"), error = function(e) 1), , FALSE))
pd <- function(...) .Internal(sprintf("%d %d", ...))
for (i in 1:3) print(lapply(list(quote(p1()), quote(p2(hc())), quote(p3()), quote(p4()), quote(p5()),
    quote(p6(1, 2)), quote(p7()), quote(pd(hc(), ))), function(e) conditionCall(tryCatch(eval(e), error = identity))))
# the same with the JIT off (closures evaluated as they are): a literal
# empty argument, one through `...`, an alias
invisible(.Internal(enableJIT(0L)))
f0 <- function() .Internal(paste0(list(side("j")), , FALSE))
g0 <- function(...) .Internal(paste0(list(...), , FALSE))
h0 <- function(...) .Internal(paste0(list("a"), ...))
a0 <- function(...) ii(paste0(list("a"), ...))
for (i in 1:2) print(list(tryCatch(f0(), error = cc), tryCatch(g0("k"), error = cc),
    tryCatch(h0(, FALSE), error = cc), tryCatch(a0(, FALSE), error = cc), h0(NULL, FALSE)))
p0 <- function() .Internal(paste0(list(hc()), , FALSE))
q0 <- function(...) .Internal(paste0(sum(...), , FALSE))
for (i in 1:2) print(lapply(list(quote(p0()), quote(q0(1, 2)), quote(pd(hc(), ))),
    function(e) conditionCall(tryCatch(eval(e), error = identity))))
# the empty argument's number counts `...` as one argument, before or after it
e1 <- function(...) .Internal(paste0(, ...))
e2 <- function(...) .Internal(paste0(..., , FALSE))
for (i in 1:2) print(list(tryCatch(e1(NULL, FALSE), error = cc), tryCatch(e2(list("a"), NULL), error = cc)))
