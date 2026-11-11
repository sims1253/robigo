# The argument checks a primitive's C function opens with (issue #280):
# checkArity ("N argument(s) passed to 'f' which requires K") and
# check1arg ("supplied argument name 'a' does not match 'x'"), in GNU R's
# order, plus the primitives that check differently (`!` dispatches
# first, as.raw names before arity, unary arithmetic, .C's check1arg2,
# Exec's matchArgs, invisible, trunc).
p <- function(expr) {
  r <- tryCatch({ v <- expr; paste("OK", deparse(v)[1L]) },
                error = function(e) paste("ERR", conditionMessage(e)),
                warning = function(w) paste("WARN", conditionMessage(w)))
  cat(deparse(substitute(expr))[1L], "::", r, "\n")
}
# the issue's three calls
p(switch(a = 1L, EXPR = "a")); p(substitute(e = x)); p(length(xx = 1L))
# checkArity, then check1arg
p(length(1, 2)); p(length()); p(length(x = 1:3)); p(`length<-`(y = 1, 2))
p(exp(y = 1)); p(exp(1, 2)); p(exp(x = 0)); p(abs(1, 2)); p(gamma(z = 1))
p(log2(y = 4)); p(log10(1, 2)); p(Re(x = 1i)); p(Conj(z = 1i))
p(is.null(y = 1)); p(is.numeric(1, 2)); p(is.array(y = 1)); p(is.single(y = 1))
p(is.single(1)); p(isS4(y = 1)); p(is.na(1, 2)); p(is.finite(y = 1))
p(dim(y = 1)); p(dimnames(1, 2)); p(names(y = 1)); p(attributes(y = 1))
p(class(y = 1)); p(oldClass(y = 1)); p(`class<-`(y = 1, "a")); p(unclass(1, 2))
p(as.environment(y = 1)); p(as.call(y = list(sum))); p(pos.to.env(y = 1))
p(seq_len(n = 3)); p(seq_along(x = 1:3)); p(quote(e = 1)); p(quote(1, 2))
p(xtfrm(y = 1)); p(enc2utf8(y = "a")); p(tracemem(y = 1)); p(...length(1))
# arity only
p(`::`(base)); p(`:::`(base, a, b)); p(cumsum(1, 2)); p(`$`(list(a = 1)))
p(`attr<-`(1, "a")); p(baseenv(1)); p(emptyenv(1)); p(globalenv(1))
p(`environment<-`(1)); p(`oldClass<-`(1)); p(`dim<-`(1)); p(proc.time(1))
p(nargs(zzz = 1)); p(interactive(1)); p(.Primitive("sum", 2))
# `!`: Ops dispatch, then checkArity
p(`!`(TRUE, FALSE)); p(`!`()); p(`!`(x = TRUE))
# as.raw: check1arg, dispatch, then checkArity
p(as.raw(y = 1, 2)); p(as.raw(environment(), 2)); p(as.raw()); p(as.raw(environment()))
# unary arithmetic other than + and -
p(`*`(1)); p(`*`("a")); p(`*`(1i)); p(`/`(TRUE)); p(`^`(2L)); p(`%%`(1)); p(`%/%`(1))
p(`*`()); p(`*`(1, 2, 3))
# trunc: Math dispatch, then check1arg (arity -1)
p(trunc()); p(trunc(y = 1)); p(trunc(1.5, 2)); p(trunc(x = 1.5))
# nzchar, invisible
p(nzchar(y = "a")); p(nzchar("a", TRUE, 3)); p(nzchar())
p(invisible(zzz = 1)); p(invisible(1, 2)); p(invisible(zzz = 1, 2)); p(invisible())
# .C and friends: check1arg2
p(.C(NAOK = TRUE)); p(.Call()); p(.External(zzz = "f")); p(.Call(PACKAGE = "base"))
p(.Fortran(name = "f")); p(.External2(.NAME = "f"))
# Exec and Tailcall
p(Exec()); p(Exec(quote(1))); p(Exec(expr = quote(1 + 1))); p(Exec(zzz = 1))
p(Exec(expression(2 + 2)))
f1 <- function() Exec(quote(1 + 2)); p(f1())
f2 <- function() { on.exit(NULL); Exec(quote(3 + 4)) }; p(f2())
f3 <- function(n) if (n == 0) "done" else Tailcall(f3, n - 1); p(f3(5))
p(Tailcall()); p(Tailcall(zzz = sum, 1))
# their missing-argument errors are missingArgError subclasses raised
# against the lexical call (getLexicalCall), after both of Exec's matched
# arguments are evaluated
q <- function(expr) {
  e <- tryCatch(expr, error = function(e) e)
  cat(deparse(substitute(expr))[1L], "::", class(e), "::", deparse(conditionCall(e))[1L], "\n")
}
q(Exec()); q(Tailcall())
g1 <- function(a) Exec(); q(g1(1))
g2 <- function(a) { x <- 1; Tailcall() }; q(g2(1))
g3 <- function() local(Exec()); q(g3())
print(tryCatch(Exec(), missingArgError = function(e) "caught"))
print(tryCatch(Exec(), tailcallError = function(e) "caught"))
print(tryCatch(Tailcall(), tailcallRecError = function(e) "caught"))
p(Exec(envir = stop("boom"))); p(Exec(quote(1 + 1), envir = stop("boom")))
# forceAndCall with too few arguments
p(forceAndCall(zzz = 1)); p(forceAndCall())
# missing/on.exit/UseMethod/return
f <- function(a) missing(zzz = a); p(f())
g <- function(a) missing(a, a); p(g())
h <- function() on.exit(zzz = 1); p(h())
p(UseMethod(zzz = "f"))
r1 <- function() return(zzz = 1); p(r1())
