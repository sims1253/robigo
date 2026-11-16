# check_stack_balance through closures: the AST interpreter checks, byte
# code checks nothing (a small closure is compiled at its second call), and
# AST `return()` leaves by a longjmp that restores the protect stack.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/ppleak.so"))
mark <- function(s) cat("##", s, "\n", file = stderr())
f0 <- function() .Call("leak")
mark("closure1"); f0()
mark("closure2"); f0()
mark("closure3"); f0()
f1 <- function() { .Call("leak") }
mark("closure-brace"); f1()
f2 <- function(x) x
mark("promise"); f2(.Call("leak"))
f3 <- function(x) { x }
mark("promise-brace"); f3(.Call("leak"))
f4 <- function() { y <- .Call("leak"); y }
mark("closure-assign"); f4()
f5 <- function() return(.Call("leak"))
mark("return"); f5()
fr <- function() { .Call("leak"); return(1) }
mark("return-erase"); invisible(fr())
mark("return-compiled"); invisible(fr())
f <- function(x) x
mark("nested-promise"); f(f(.Call("leak")))
mark("anonymous"); (function() .Call("leak"))()
h <- function(n) if (n > 0) h(n - 1) else .Call("leak")
mark("h0"); h(0)
mark("h1"); h(1)
mark("h3"); h(3)
k <- function(n) if (n > 0) k(n - 1) else .Call("leak")
mark("k3-first"); k(3)
# a small closure made in a local frame is never compiled
mark("local-recursion"); local({ r <- function(n) if (n > 0) r(n - 1) else .Call("leak"); r(3) })
mark("end")
