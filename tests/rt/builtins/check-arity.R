# util.c's checkArity, one port for every primitive: an .Internal's
# "%d argument(s) passed to .Internal(%s) which requires %d" is error()'s,
# reported against the context's call; a primitive's "... passed to '%s'
# which requires %d" is errorcall()'s, against its own call.
m <- function(e) tryCatch(e, error = function(c) {
  cat(conditionMessage(c), "\n")
  print(conditionCall(c))
})
m(.Internal(stop(TRUE)))
m(.Internal(geterrmessage(1)))
m(.Internal(mem.maxVSize(1, 2)))
m(.Internal(eval(1)))
m(.Internal(gctorture()))
m(`(`(1, 2))
m(.Primitive("c", 1))
m(`environment<-`(m))
m(`::`(base))
m(`$`(list(a = 1)))
m(quote(1, 2))
f <- function() .Internal(stop(TRUE))
m(f())
g <- function() `(`()
m(g())
# do_maxVSize makes no check: asReal(CAR(args)), the rest ignored
m(.Internal(mem.maxVSize(Inf, 2)))
m(.Internal(mem.maxVSize()))
