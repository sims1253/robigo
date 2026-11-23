# R_getVar and R_getVarEx from package C (envir.c): the value from the
# environment or (inherits) its enclosures, a promise forced once, an
# unbound symbol R_getVar's error or R_getVarEx's ifnotfound, the type
# checks' errors (both name R_getVarEx), and a missing argument's
# missingArgError of subclass getVarExError, reported against the
# environment's function call. R_getVar's not-found check is on the
# value after forcing, and a C NULL fallback is returned as it is.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/get_var.so"))
gv <- function(sym, rho, inherits = TRUE) .Call("rt_get_var", sym, rho, inherits)
gve <- function(sym, rho, inherits = TRUE, nf = NULL) .Call("rt_get_var_ex", sym, rho, inherits, nf)
show <- function(expr) {
  e <- tryCatch(expr, error = function(e) e)
  if (inherits(e, "condition")) {
    print(class(e))
    print(conditionMessage(e))
    print(conditionCall(e))
  } else print(e)
}
x <- 1
e <- new.env()
assign("y", 2, envir = e)
gv(quote(y), e)
gv(quote(x), e)
gve(quote(x), e, FALSE, "not here")
gve(quote(nosuch), e, TRUE, NULL)
show(gv(quote(x), e, FALSE))
show(gv(quote(nosuch), e))
delayedAssign("p", {cat("forced\n"); 42}, assign.env = e)
gv(quote(p), e)
gv(quote(p), e)
show(gv("y", e))
show(gv(quote(y), list(y = 1)))
show(gve(1, e))
f <- function(a) gve(quote(a), environment())
show(f())
g <- function(a) gv(quote(a), environment(), FALSE)
show(g())
tryCatch(f(), getVarExError = function(e) cat("getVarExError caught\n"))
tryCatch(f(), missingArgError = function(e) cat("missingArgError caught\n"))
f(3)
# R_getVar checks the value after forcing: a promise that gives
# R_UnboundValue is "not found"; R_getVarEx's C NULL fallback comes back
delayedAssign("u", .Call("rt_unbound"), assign.env = e)
show(gv(quote(u), e))
.Call("rt_get_var_ex_null", quote(nosuch), e)
.Call("rt_get_var_ex_null", quote(y), e)
