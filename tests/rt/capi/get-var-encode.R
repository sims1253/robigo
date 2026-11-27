# R_getVar's "object '%s' not found" and R_getVarEx's missing argument
# (getVarExError) from package C: both name the symbol by
# EncodeChar(PRINTNAME(sym)) (envir.c:2622, 2638), control characters
# escaped, and in a C locale non-ASCII bytes too.
# This copy runs under C.UTF-8 (the default; its -c copy runs under C).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/get_var.so"))
gv <- function(sym, rho, inherits = TRUE) .Call("rt_get_var", sym, rho, inherits)
gve <- function(sym, rho, inherits = TRUE, nf = NULL) .Call("rt_get_var_ex", sym, rho, inherits, nf)
show <- function(expr) tryCatch(expr, error = function(e) { print(class(e)); print(conditionMessage(e)) })
syms <- list(as.name(paste0("a", intToUtf8(10L), "b")), as.name(paste0("x", intToUtf8(1L), "y")), quote(`café`))
for (s in syms) {
  show(gv(s, globalenv()))
  a <- alist(x = ); names(a) <- as.character(s)
  f <- eval(call("function", as.pairlist(a), call("gve", call("quote", s), quote(environment()))))
  show(f())
}
