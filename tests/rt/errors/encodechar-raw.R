# Messages whose C has CHAR(PRINTNAME(sym)), not EncodeChar: the
# evaluator's missing argument (evalError), get's missing argument
# (getMissingError) and get's "of mode" message; and a long replacement
# function name's "overlong name in '%s'", which does encode (eval.c:3305)
# This copy runs under LC_ALL=C.UTF-8 (the default; its -c copy runs under C).
show <- function(expr) tryCatch(withCallingHandlers(expr, warning = function(w) {
    print(conditionMessage(w)); invokeRestart("muffleWarning") }),
  error = function(e) print(conditionMessage(e)))
syms <- list(as.name(paste0("a", intToUtf8(10L), "b")), as.name(paste0("a", intToUtf8(9L), "b")),
             as.name(paste0("x", intToUtf8(1L), "y")), quote(`café`))
for (s in syms) {
  nm <- as.character(s)
  a <- alist(x = ); names(a) <- nm
  show(eval(call("function", as.pairlist(a), s))())
  show(eval(call("function", as.pairlist(a), call("get", nm)))())
  show(get(nm, mode = "function"))
}
x <- 1
long <- as.name(paste0(strrep("f", 30L), intToUtf8(10L)))
show(eval(call("<-", call(as.character(long), quote(x)), 2)))
