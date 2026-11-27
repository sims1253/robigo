# `::`'s "'%s' is not an exported object from 'namespace:%s'" with
# EncodeChar(PRINTNAME(name)) (envir.c:4434); `:::`'s and base's `::`
# "object '%s' not found" (envir.c:4335)
# This copy runs under LC_ALL=C.UTF-8 (the default; its -c copy runs under C).
show <- function(expr) tryCatch(withCallingHandlers(expr, warning = function(w) {
    print(conditionMessage(w)); invokeRestart("muffleWarning") }),
  error = function(e) print(conditionMessage(e)))
syms <- list(as.name(paste0("a", intToUtf8(10L), "b")), as.name(paste0("a", intToUtf8(9L), "b")),
             as.name(paste0("x", intToUtf8(1L), "y")), quote(`café`))
for (s in syms) {
  show(eval(call("::", quote(compiler), s)))
  show(eval(call(":::", quote(compiler), s)))
  show(eval(call("::", quote(base), s)))
}
