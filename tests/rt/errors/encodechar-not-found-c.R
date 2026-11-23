# "object '%s' not found" with EncodeChar(PRINTNAME(sym)) (eval.c:1179,
# 2595, 5785; envir.c:2354, 2482, 4335; saveload.c:2417; serialize.c:3256's
# EncodeChar of the name): control characters escaped, and in a C locale
# non-ASCII bytes too
# This copy runs under LC_ALL=C (its .locale sidecar).
show <- function(expr) tryCatch(withCallingHandlers(expr, warning = function(w) {
    print(conditionMessage(w)); invokeRestart("muffleWarning") }),
  error = function(e) print(conditionMessage(e)))
syms <- list(as.name(paste0("a", intToUtf8(10L), "b")), as.name(paste0("a", intToUtf8(9L), "b")),
             as.name(paste0("x", intToUtf8(1L), "y")), quote(`café`))
for (s in syms) {
  nm <- as.character(s)
  show(eval(s))
  show(compiler::cmpfun(eval(call("function", NULL, s)))())
  show(get(nm))
  show(rm(list = nm))
  show(eval(call(":::", quote(base), s)))
  show(eval(call("<-", call("[", s, 1), 1)))
  show(.Internal(getVarsFromFrame(nm, globalenv(), FALSE)))
  con <- file(tempfile(), "wb")
  show(.Internal(saveToConn(nm, con, FALSE, NULL, globalenv(), FALSE)))
  close(con)
}
