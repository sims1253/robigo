# "could not find function \"%s\"" with EncodeChar(PRINTNAME(symbol))
# (envir.c:1930), interpreted and compiled
# This copy runs under LC_ALL=C.UTF-8 (the default; its -c copy runs under C).
show <- function(expr) tryCatch(withCallingHandlers(expr, warning = function(w) {
    print(conditionMessage(w)); invokeRestart("muffleWarning") }),
  error = function(e) print(conditionMessage(e)))
syms <- list(as.name(paste0("a", intToUtf8(10L), "b")), as.name(paste0("a", intToUtf8(9L), "b")),
             as.name(paste0("x", intToUtf8(1L), "y")), quote(`café`))
for (s in syms) {
  show(eval(as.call(list(s))))
  show(compiler::cmpfun(eval(call("function", NULL, as.call(list(s)))))())
}
