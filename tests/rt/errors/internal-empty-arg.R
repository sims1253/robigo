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
