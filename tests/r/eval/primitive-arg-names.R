# Primitives check supplied argument names (check1arg) and arity
# (checkArity) as GNU R does; the issue's three cases (#280) and the rest
# of a scan over names.c's one- and zero-argument primitives.
f <- function(e) {
  r <- tryCatch(eval(e), error = function(c) c)
  if (inherits(r, "error")) cat("Error in", deparse(conditionCall(r)), ":", conditionMessage(r), "\n")
  else print(r)
}
f(quote(switch(a = 1L, EXPR = "a")))
f(quote(substitute(e = x)))
f(quote(length(xx = 1L)))
f(quote(log2(y = 4)))
f(quote(log10(y = 4)))
f(quote(`class<-`(y = 1, "a")))
f(quote(is.array(y = 1)))
f(quote(is.single(y = 1)))
f(quote(as.environment(y = 1)))
f(quote(as.environment()))
f(quote(nargs(y = 1)))
f(quote(interactive(1)))
f(quote(emptyenv(1)))
f(quote(baseenv(1, 2)))
f(quote(globalenv(y = 1)))
f(quote(proc.time(1)))
f(quote((function(...) ...length(1))()))
f(quote((function(...) ...names(1))()))
# the unnamed and matching-name forms still work
f(quote(log2(x = 4)))
f(quote(is.array(x = matrix(1))))
f(quote(class(`class<-`(x = 1, "a"))))
f(quote(environmentName(as.environment(x = 1))))
f(quote((function(...) c(...length(), nargs()))(1, 2)))
g <- function(a = 1) { a; class(a) <- "k"; a }
f(quote(g()))
