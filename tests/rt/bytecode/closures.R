# Closures of byte code (eval.c:do_bcclose): the formals are checked
# (CheckFormals), the body must be byte code, the environment one. The
# closure's body() and deparse() are its source (the first constant,
# bytecodeExpr), bodyCode() the byte code; identical() ignores byte code
# unless told not to; `environment<-` returns it to its source. Called,
# such a closure runs its source until V3 translates byte code.
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
b <- .Internal(mkCode(c(12L, 16L, 0L, 1L), list(42L)))
print(err(.Internal(bcClose(list(x = 1), b, globalenv()))))
print(err(.Internal(bcClose(pairlist(1), b, globalenv()))))
print(err(.Internal(bcClose(NULL, 42L, globalenv()))))
print(err(.Internal(bcClose(NULL, b, NULL))))
print(err(.Internal(bcClose(NULL, b, 1))))
local({
  f <- .Internal(bcClose(pairlist(x = 1L), b, globalenv()))
  print(typeof(f))
  print(f())
  print(f(2L))
  print(body(f))
  print(formals(f))
  print(typeof(.Internal(bodyCode(f))))
  print(identical(.Internal(bodyCode(f)), b))
  print(deparse(f))
  g <- function(x = 1L) 42L
  print(identical(f, g))
  print(identical(f, g, ignore.bytecode = FALSE))
  print(identical(f, f, ignore.bytecode = FALSE))
  h <- f
  environment(h) <- new.env()
  print(typeof(.Internal(bodyCode(h))))
  print(body(h))
  print(typeof(.Internal(bodyCode(f))))
  print(.Internal(bodyCode(function(x) x)))
  print(.Internal(bodyCode(sum)))
  print(body(sum))
  print(err(body(1)))
  e <- new.env()
  assign("x", 5, envir = e)
  k <- .Internal(bcClose(NULL, .Internal(mkCode(c(12L, 16L, 0L, 1L), list(quote(x + 1)))), e))
  print(body(k))
  print(environment(k)$x)
  invisible(NULL)
})
